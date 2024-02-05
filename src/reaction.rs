use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

use indexmap::IndexMap;
use once_cell::sync::Lazy;
use regex::Regex;
use tokio::{
    sync::mpsc::{channel, Sender},
    time::sleep,
};
use volty::{http::routes::servers::member_edit::MemberEdit, prelude::*};

use crate::{error::Error, Bot};

#[derive(Clone, Debug)]
pub struct SetupMessage {
    author_id: String,
    content: String,
    roles: Vec<(Range<usize>, String)>,
    is_formatted: bool,
}

impl SetupMessage {
    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn parse(author_id: String, mut content: &str) -> Option<Self> {
        content = content.trim();
        let mut is_exclusive = false;
        let mut is_formatted = false;
        for word in content.split_whitespace() {
            if word.eq_ignore_ascii_case("exclusive") {
                content = &content["exclusive".len()..].trim_start();
                is_exclusive = true;
            } else if word.eq_ignore_ascii_case("formatted") {
                content = &content["formatted".len()..].trim_start();
                is_formatted = true;
            } else {
                break;
            }
        }
        let content = if is_exclusive {
            format!("[](EXCLUSIVE){content}")
        } else {
            content.to_string()
        };
        static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\{ROLE:([^{}]{1,32})}").unwrap());
        let captures = RE.captures_iter(&content);
        let mut roles = Vec::new();
        for capture in captures {
            let range = capture.get(0).unwrap().range();
            let name_or_id = capture.get(1).unwrap().as_str();
            roles.push((range, name_or_id.into()));
        }
        if roles.is_empty() {
            return None;
        }
        Some(Self {
            author_id,
            content,
            roles,
            is_formatted,
        })
    }

    pub fn with_emojis(&self, emojis: &[&str], server: &Server) -> Option<String> {
        if emojis.is_empty() {
            return Some(self.content.clone());
        }
        let mut with_emojis = String::with_capacity(self.content.len());
        let mut role = &self.roles[0];
        let mut role_index = 0;
        for (i, c) in self.content.char_indices() {
            if i >= role.0.end {
                role_index += 1;
                if role_index < emojis.len() && role_index < self.roles.len() {
                    role = &self.roles[role_index];
                } else {
                    with_emojis.push_str(&self.content[i..]);
                    break;
                }
            }
            if i < role.0.start {
                with_emojis.push(c);
                continue;
            }
            if i == role.0.start {
                let emoji = emojis[role_index];
                let emoji = emojis::get(emoji)
                    .and_then(emojis::Emoji::shortcode)
                    .unwrap_or(emoji);
                let (role_id, role) = server.role_by_id_or_name(&role.1)?;
                if self.is_formatted {
                    with_emojis.push_str(&format!(":{emoji}:[]({role_id})"));
                } else {
                    with_emojis.push_str(&format!(":{emoji}:[]({role_id}) __{}__", role.name));
                }
                continue;
            }
        }
        Some(with_emojis)
    }
}

#[derive(Clone, Debug)]
pub struct RoleMessage {
    exclusive: bool,
    // k=Emoji, v=RoleID
    roles: HashMap<String, String>,
}

impl RoleMessage {
    fn parse(content: &str) -> Option<Self> {
        static RE: Lazy<Regex> = Lazy::new(|| {
            Regex::new(r"(?i):([a-z0-9_-]+):\[]\(([0-9A-HJKMNP-TV-Z]{26})\)").unwrap()
        });
        let captures = RE.captures_iter(content);
        let mut roles = HashMap::new();
        for capture in captures {
            let emoji = capture.get(1).unwrap().as_str();
            let role_id = capture.get(2).unwrap().as_str();
            roles.insert(emoji.to_string(), role_id.to_string());
        }
        if roles.is_empty() {
            return None;
        }
        let exclusive = content.starts_with("[](EXCLUSIVE)");
        Some(Self { exclusive, roles })
    }
}

pub enum RoleReact {
    React,
    Unreact,
}

#[derive(Clone)]
pub struct RoleAction {
    give: Vec<String>,
    remove: Vec<String>,
}

pub type ServerSender = Sender<(String, RoleAction)>;

impl Bot {
    async fn queue_edit(&self, server_id: &str, user_id: String, action: RoleAction) {
        let handlers = self.server_handlers.read().await;
        if let Some(sender) = handlers.get(server_id) {
            match sender.send((user_id.clone(), action.clone())).await {
                Ok(_) => return,
                Err(e) => {
                    dbg!(e);
                }
            };
        }
        drop(handlers);
        let cache = self.cache.clone();
        let http = self.http.clone();
        let (tx, mut rx) = channel(100);
        let server_id = server_id.to_string();
        let server_id_ = server_id.clone();
        tokio::spawn(async move {
            let mut next: Option<(String, RoleAction)> = None;
            let mut edits: IndexMap<String, HashSet<String>> = IndexMap::new();
            'outer: loop {
                while let Some((user_id, action)) = next {
                    if !edits.contains_key(&user_id) {
                        let member = cache
                            .fetch_member(&http, &server_id, &user_id)
                            .await
                            .unwrap();
                        edits.insert(user_id.clone(), member.roles);
                    }
                    let edit = edits.get_mut(&user_id).unwrap();
                    edit.extend(action.give);
                    for role in action.remove {
                        edit.remove(&role);
                    }
                    next = rx.try_recv().ok();
                }

                for (user_id, roles) in &edits {
                    let member = cache
                        .fetch_member(&http, &server_id, user_id)
                        .await
                        .unwrap();
                    if *roles == member.roles {
                        continue;
                    };
                    let giving = roles.difference(&member.roles);
                    let taking = member.roles.difference(roles);
                    println!("Server: {server_id}, Member: {user_id}\n\tGiving: {giving:?}\n\tTaking: {taking:?}");
                    let data = MemberEdit::new().roles(roles);
                    let result = http.edit_member(&server_id, user_id, data).await;
                    match result {
                        Err(HttpError::Api(ApiError::RetryAfter(duration))) => {
                            println!("RetryAfter: {duration:?}");
                            sleep(duration).await;
                            if let Some(index) = edits.get_index_of(user_id) {
                                if index > 0 {
                                    edits.drain(0..index);
                                }
                            }
                            next = rx.try_recv().ok();
                            continue 'outer;
                        }
                        Err(e) => {
                            dbg!(e);
                        }
                        _ => {}
                    }
                }
                edits.clear();
                next = rx.recv().await;
                if next.is_none() {
                    return;
                }
            }
        });
        if let Err(e) = tx.send((user_id, action)).await {
            dbg!(e);
        }
        let mut handlers = self.server_handlers.write().await;
        handlers.insert(server_id_, tx);
    }

    async fn role_message(
        &self,
        channel_id: &str,
        message_id: &str,
    ) -> Result<Option<RoleMessage>, HttpError> {
        if let Some(role_message) = self.role_messages.read().await.get(message_id) {
            return Ok(Some(role_message.clone()));
        }
        let message = self
            .cache
            .fetch_message(&self.http, channel_id, message_id)
            .await?;
        let role_message = message.content.as_ref().and_then(|c| RoleMessage::parse(c));
        if let Some(message) = role_message.as_ref() {
            self.role_messages
                .write()
                .await
                .insert(message_id.to_string(), message.clone());
        }
        Ok(role_message)
    }

    async fn check_role_message(
        &self,
        server_id: &str,
        author_id: &str,
        role_message: &RoleMessage,
    ) -> Result<(), Error> {
        let role_ids = role_message.roles.values().map(String::as_str);
        self.check_above_roles(server_id, self.cache.user_id(), role_ids.clone())
            .await?;
        self.check_above_roles(server_id, author_id, role_ids)
            .await?;
        Ok(())
    }

    async fn setup_message(
        &self,
        channel_id: &str,
        message_id: &str,
    ) -> Result<Option<SetupMessage>, HttpError> {
        if let Some(setup_message) = self.setup_messages.read().await.get(message_id) {
            return Ok(Some(setup_message.clone()));
        }
        let bot_message = self
            .cache
            .fetch_message(&self.http, channel_id, message_id)
            .await?;
        let Some(replies) = bot_message.replies else {
            return Ok(None);
        };
        let Some(reply) = replies.first() else {
            return Ok(None);
        };
        let user_message = self
            .cache
            .fetch_message(&self.http, channel_id, reply)
            .await?;
        let setup_message = user_message
            .content
            .as_ref()
            .and_then(|c| c.strip_prefix(self.cache.user_mention()))
            .and_then(|c| SetupMessage::parse(user_message.author_id, c));
        if let Some(message) = setup_message.as_ref() {
            self.setup_messages
                .write()
                .await
                .insert(message_id.to_string(), message.clone());
        }
        Ok(setup_message)
    }

    pub async fn check_setup_message(
        &self,
        server_id: &str,
        author_id: &str,
        setup_message: &SetupMessage,
    ) -> Result<(), Error> {
        self.check_server_perms(server_id, self.cache.user_id(), &[Permission::AssignRoles])
            .await?;
        self.check_server_perms(server_id, author_id, &[Permission::AssignRoles])
            .await?;

        let ids_or_names = setup_message.roles.iter().map(|(_, i)| i.as_str());
        self.check_above_roles(server_id, self.cache.user_id(), ids_or_names.clone())
            .await?;
        self.check_above_roles(server_id, author_id, ids_or_names)
            .await?;

        Ok(())
    }

    pub async fn on_react(
        &self,
        channel_id: &str,
        message_id: &str,
        user_id: &str,
        emoji_id: &str,
        action: RoleReact,
    ) -> Result<(), Error> {
        let message = self
            .cache
            .fetch_message(&self.http, channel_id, message_id)
            .await?;
        if message.author_id != self.cache.user_id() {
            return Ok(());
        }
        let Some(interactions) = &message.interactions else {
            return Ok(());
        };
        if interactions.restrict_reactions {
            self.on_role_react(channel_id, message_id, user_id, emoji_id, action)
                .await?;
        } else if message.replies.is_some() {
            self.on_setup_react(message, user_id).await?;
        }
        Ok(())
    }

    pub async fn on_react_error(&self, channel_id: &str, user_id: &str, error: Error) {
        dbg!(&error);
        let error = match error {
            Error::Custom(message) => message,
            Error::InvalidRole(_) => "Role doesn't exist".to_string(),
            Error::Missing(permission)
            | Error::Http(HttpError::Api(ApiError::MissingPermission { permission })) => {
                format!("I don't have `{permission}` permissions!")
            }
            Error::MemberRankTooHigh => {
                "I can't assign roles to members ranked above me!".to_string()
            }
            Error::RoleRankTooHigh(role) => {
                format!("I can only assign roles below my own!\n{role}")
            }
            Error::UserMissing(_) | Error::UserRankTooLow(_) => {
                unreachable!()
            }
            Error::InvalidUser | Error::Http(_) => return,
        };

        if let Ok(dm) = self.cache.fetch_dm(&self.http, user_id).await {
            let server = self
                .get_server(channel_id)
                .await
                .map_or("Unknown".to_string(), |s| s.name);
            let content = format!("Server: {server}\nError: {error}");
            let _ = self.http.send_message(dm.id(), content).await;
        }
    }

    async fn on_role_react(
        &self,
        channel_id: &str,
        message_id: &str,
        user_id: &str,
        emoji_id: &str,
        action: RoleReact,
    ) -> Result<(), Error> {
        let emoji_id = emojis::get(emoji_id)
            .and_then(emojis::Emoji::shortcode)
            .unwrap_or(emoji_id);
        let Some(role_message) = self.role_message(channel_id, message_id).await? else {
            return Ok(());
        };
        let Some(role_id) = role_message.roles.get(emoji_id) else {
            return Ok(());
        };
        let Some(server) = self.get_server(channel_id).await else {
            return Ok(());
        };
        if !server.roles.contains_key(role_id) {
            return Err(Error::InvalidRole(role_id.clone()));
        }
        self.check_server_perms(&server.id, self.cache.user_id(), &[Permission::AssignRoles])
            .await?;

        let bot_member = self
            .cache
            .fetch_member(&self.http, &server.id, self.cache.user_id())
            .await?;
        let user_member = self
            .cache
            .fetch_member(&self.http, &server.id, user_id)
            .await?;
        let bot_rank = bot_member.rank(&server);
        let user_rank = user_member.rank(&server);
        let role = server.roles.get(role_id).unwrap();
        if bot_rank >= user_rank {
            return Err(Error::MemberRankTooHigh);
        }
        if bot_rank >= role.rank {
            return Err(Error::RoleRankTooHigh(role.name.clone()));
        }

        let action = match action {
            RoleReact::React => {
                let remove = if role_message.exclusive {
                    role_message
                        .roles
                        .values()
                        .filter(|&r| r != role_id)
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                };
                RoleAction {
                    give: vec![role_id.into()],
                    remove,
                }
            }
            RoleReact::Unreact => RoleAction {
                give: Vec::new(),
                remove: vec![role_id.into()],
            },
        };
        println!(
            "queue_edit: Server: {}, Member: {}, Give: {:?}, Take: {:?}",
            &server.id, &user_member.id.user, &action.give, &action.remove
        );
        self.queue_edit(&server.id, user_member.id.user, action)
            .await;
        Ok(())
    }

    async fn on_setup_react(&self, message: Message, user_id: &str) -> Result<(), Error> {
        let Some(setup) = self.setup_message(&message.channel_id, &message.id).await? else {
            return Ok(());
        };
        if setup.author_id != user_id {
            return Err(Error::InvalidUser);
        }
        let mut is_checkmarked = false;
        let mut emojis = Vec::with_capacity(setup.roles.len());
        for (emoji, user_ids) in &message.reactions {
            if user_ids.contains(user_id) {
                if emoji == "✅" {
                    is_checkmarked = true;
                    continue;
                }
                if emojis.len() < setup.roles.len() {
                    emojis.push(emoji.as_str());
                };
            }
        }
        let channel = self.cache.get_channel(&message.channel_id).await.unwrap();
        let server = self
            .cache
            .get_server(channel.server_id().unwrap())
            .await
            .unwrap();
        if let Some(mut content) = setup.with_emojis(&emojis, &server) {
            if content.len() > 2_000 {
                if content.is_char_boundary(2_000) {
                    content.truncate(2_000);
                } else {
                    let new_len = content
                        .char_indices()
                        .rev()
                        .map(|(index, _)| index)
                        .find(|index| *index < 2_000)
                        .unwrap_or(0);
                    content.truncate(new_len);
                }
            }
            let is_complete = is_checkmarked && emojis.len() == setup.roles.len();
            if !is_complete {
                self.http
                    .edit_message(&message.channel_id, &message.id, content)
                    .await?;
            } else {
                self.setup_messages.write().await.remove(&message.id);
                let Some(role_message) = RoleMessage::parse(&content) else {
                    return Ok(());
                };
                self.check_role_message(&server.id, user_id, &role_message)
                    .await?;

                let _ = self
                    .http
                    .delete_message(&message.channel_id, &message.id)
                    .await;
                let reply = SendableMessage::new()
                    .content(content)
                    .interactions(Interactions::new(emojis).restrict());
                let response = self.http.send_message(message.channel_id, reply).await?;
                self.role_messages
                    .write()
                    .await
                    .insert(response.id, role_message);
            }
        }
        Ok(())
    }
}
