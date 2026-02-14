use std::{collections::HashMap, fmt::Write, sync::Arc};

use database::ServerSettings;
use once_cell::sync::Lazy;
use reaction::{RoleMessage, RoleReact, ServerSender, SetupMessage};
use regex::Regex;
use tokio::sync::RwLock;
use volty::{
    http::routes::{servers::role_edit::RoleEdit, users::user_edit::UserEdit},
    prelude::*,
    types::{servers::server::FieldsRole, util::regex::RE_ROLE_MENTION},
};

mod autorole;
mod constants;
mod database;
mod error;
mod reaction;

use constants::*;
use error::Error;

use crate::database::SqliteDB;

fn parse_colours(colours: &str) -> String {
    let colours = colours.trim();
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^(#?[a-z0-9]+)$").unwrap());
    if colours.split_whitespace().all(|a| RE.is_match(a)) {
        let colours: Vec<&str> = colours.split_whitespace().collect();
        if colours.len() > 1 {
            return format!("linear-gradient(to right,{})", colours.join(","));
        }
    }
    colours.to_string()
}

struct Bot {
    http: Http,
    cache: Cache,
    db: SqliteDB,

    setup_messages: RwLock<HashMap<String, SetupMessage>>,
    role_messages: RwLock<HashMap<String, RoleMessage>>,

    server_handlers: RwLock<HashMap<String, ServerSender>>,
}

impl Bot {
    async fn check_server_perms(
        &self,
        server_id: &str,
        user_id: &str,
        permissions: &[Permission],
    ) -> Result<(), Error> {
        let perms = self
            .cache
            .fetch_server_permissions(&self.http, server_id, user_id)
            .await?;
        let my_id = self.cache.user_id();
        for &permission in permissions {
            if !perms.has(permission) {
                return if user_id == my_id {
                    Err(Error::Missing(permission))
                } else {
                    Err(Error::UserMissing(permission))
                };
            }
        }
        Ok(())
    }

    async fn check_above_roles(
        &self,
        server_id: &str,
        user_id: &str,
        role_ids_or_names: impl IntoIterator<Item = &str>,
    ) -> Result<(), Error> {
        let server = self.cache.get_server(server_id).await.unwrap();
        let member = self
            .cache
            .fetch_member(&self.http, server_id, user_id)
            .await?;
        let rank = member.effective_rank(&server);
        let my_id = self.cache.user_id();
        for id_or_name in role_ids_or_names {
            let Some((_role_id, role)) = server.role_by_id_or_name(id_or_name) else {
                return Err(Error::InvalidRole(id_or_name.to_string()));
            };
            if role.rank <= rank {
                return if user_id == my_id {
                    Err(Error::RoleRankTooHigh(role.name.clone()))
                } else {
                    Err(Error::UserRankTooLow(role.name.clone()))
                };
            }
        }
        Ok(())
    }

    async fn get_server(&self, channel_id: &str) -> Option<Server> {
        let channel = self.cache.get_channel(channel_id).await?;
        self.cache.get_server(channel.server_id()?).await
    }

    async fn on_message(&self, message: &Message) -> Result<(), Error> {
        if message.author_id == self.cache.user_id() {
            return Ok(());
        }
        let Some(content) = message
            .content
            .as_ref()
            .and_then(|c| c.strip_prefix(self.cache.user_mention()))
        else {
            return Ok(());
        };
        let Some(server) = self.get_server(&message.channel_id).await else {
            return Ok(());
        };

        let user_id = &message.author_id;
        let user = self.cache.fetch_user(&self.http, user_id).await?;
        if user.bot.is_some() {
            return Ok(());
        }

        let bot_permissions = self
            .cache
            .fetch_channel_permissions(&self.http, &message.channel_id, self.cache.user_id())
            .await?;
        if !bot_permissions.has(Permission::SendMessage) {
            return Err(Error::Missing(Permission::SendMessage));
        }

        let content = content.trim();
        let (command, rest) = content
            .split_once(char::is_whitespace)
            .unwrap_or((content, ""));
        let rest = rest.trim_start();
        match command.to_lowercase().as_str() {
            "" | "help" => {
                return self.help_command(message).await;
            }
            "auto" | "autorole" => {
                return self.autorole_command(message, rest).await;
            }
            "color" | "colour" => {
                return self.colour_command(message, rest).await;
            }
            _ => {}
        }
        let Some(setup) = SetupMessage::parse(message.author_id.clone(), content) else {
            return Ok(());
        };
        self.check_setup_message(&server.id, user_id, &setup)
            .await?;

        let reply = SendableMessage::new()
            .content(setup.content())
            .interactions(Interactions::new(["✅"]))
            .reply(message.id.as_str());
        let response = self.http.send_message(&message.channel_id, reply).await?;
        self.setup_messages.write().await.insert(response.id, setup);
        Ok(())
    }

    async fn on_message_error(&self, message: &Message, error: Error) {
        let error = match error {
            Error::Custom(message) => message,
            Error::InvalidRole(role) => {
                format!("Role not found!\n{role}")
            }
            Error::Missing(permission)
            | Error::Http(HttpError::Api(ApiError::MissingPermission { permission })) => {
                let error = format!("I don't have `{permission}` permissions!");
                if permission == Permission::SendMessage {
                    if let Ok(dm) = self.cache.fetch_dm(&self.http, &message.author_id).await {
                        let server = self
                            .get_server(&message.channel_id)
                            .await
                            .map_or("Unknown".to_string(), |s| s.name);
                        let content = format!("Server: {server}\nError: {error}");
                        let _ = self.http.send_message(dm.id(), content).await;
                    }
                    return;
                }
                error
            }
            Error::UserMissing(permission) => {
                format!("You don't have `{permission}` permissions!")
            }
            Error::RoleRankTooHigh(role) => {
                format!("I can only assign roles below my own!\n{role}")
            }
            Error::UserRankTooLow(role) => {
                format!("You can only assign roles below your own!\n{role}")
            }
            Error::MemberRankTooHigh | Error::InvalidUser => unreachable!(),
            Error::Http(_) => return,
        };

        let _ = self.http.send_message(&message.channel_id, error).await;
    }

    async fn help_command(&self, message: &Message) -> Result<(), Error> {
        self.http
            .send_message(
                &message.channel_id,
                HELP_MESSAGE.replace("%BOT_MENTION%", self.cache.user_mention()),
            )
            .await?;
        Ok(())
    }

    async fn colour_command(&self, message: &Message, args: &str) -> Result<(), Error> {
        if args.is_empty() {
            self.http
                .send_message(
                    &message.channel_id,
                    HELP_COLOUR_MESSAGE.replace("%BOT_MENTION%", self.cache.user_mention()),
                )
                .await?;
            return Ok(());
        }
        let (mut role_id_or_name, rest) =
            args.split_once(char::is_whitespace).unwrap_or((args, ""));
        let Some(server) = self.get_server(&message.channel_id).await else {
            return Ok(());
        };
        if let Some(role_id) = RE_ROLE_MENTION
            .captures(role_id_or_name)
            .map(|c| c.get(1).unwrap().as_str())
        {
            role_id_or_name = role_id;
        }
        let Some((role_id, _role)) = server.role_by_id_or_name(role_id_or_name) else {
            return Err(Error::InvalidRole(role_id_or_name.to_string()));
        };

        self.check_server_perms(&server.id, self.cache.user_id(), &[Permission::ManageRole])
            .await?;
        self.check_server_perms(&server.id, &message.author_id, &[Permission::ManageRole])
            .await?;

        self.check_above_roles(&server.id, self.cache.user_id(), [role_id_or_name])
            .await?;
        self.check_above_roles(&server.id, &message.author_id, [role_id_or_name])
            .await?;

        let colour = parse_colours(rest);
        if colour.len() > 128 {
            return Err(Error::Custom(format!(
                "Colour must be 128 characters or less!\n{colour}"
            )));
        }
        let edit = if colour.is_empty() {
            RoleEdit::new().remove(FieldsRole::Colour)
        } else {
            RoleEdit::new().colour(colour)
        };
        self.http.edit_role(&server.id, role_id, edit).await?;
        self.http
            .send_message(&message.channel_id, "Role colour set!")
            .await?;
        Ok(())
    }

    async fn autorole_command(&self, message: &Message, args: &str) -> Result<(), Error> {
        let Some(server) = self.get_server(&message.channel_id).await else {
            return Ok(());
        };
        if args.is_empty() {
            let mut send =
                HELP_AUTOROLE_MESSAGE.replace("%BOT_MENTION%", self.cache.user_mention());
            if let Some(settings) = self.db.get_settings(&server.id).await
                && !settings.auto_roles.is_empty()
            {
                    write!(send, "\nCurrent AutoRoles:").unwrap();

                    for role in settings.auto_roles {
                        let name = server.roles.get(&role).map(|r| &r.name).unwrap_or(&role);
                        write!(send, "\n`{name}`").unwrap();
                }
            }
            self.http.send_message(&message.channel_id, send).await?;
            return Ok(());
        }

        self.check_server_perms(&server.id, self.cache.user_id(), &[Permission::AssignRoles])
            .await?;
        self.check_server_perms(
            &server.id,
            &message.author_id,
            &[Permission::AssignRoles, Permission::ManageServer],
        )
        .await?;

        let mut settings = ServerSettings {
            id: server.id.clone(),
            auto_roles: Vec::new(),
        };
        if args != "clear" {
            for mut role_id_or_name in args.split_ascii_whitespace() {
                if let Some(role_id) = RE_ROLE_MENTION
                    .captures(role_id_or_name)
                    .map(|c| c.get(1).unwrap().as_str())
                {
                    role_id_or_name = role_id;
                }

                let Some((role_id, _role)) = server.role_by_id_or_name(role_id_or_name) else {
                    return Err(Error::InvalidRole(role_id_or_name.to_string()));
                };
                self.check_above_roles(&server.id, self.cache.user_id(), [role_id])
                    .await?;
                self.check_above_roles(&server.id, &message.author_id, [role_id])
                    .await?;

                settings.auto_roles.push(role_id.to_string());
                if settings.auto_roles.len() > 25 {
                    self.http
                        .send_message(&message.channel_id, "No more than 25 autoroles!")
                        .await?;
                    return Ok(());
                }
            }
        }
        self.db.save_settings(settings).await?;

        let send = if args == "clear" {
            "AutoRole cleared!"
        } else {
            "AutoRole set!"
        };
        self.http.send_message(&message.channel_id, send).await?;
        Ok(())
    }
}

#[async_trait]
impl RawHandler for Bot {
    async fn on_ready(
        &self,
        _users: Vec<User>,
        _servers: Vec<Server>,
        _channels: Vec<Channel>,
        _members: Vec<Member>,
        _emojis: Vec<Emoji>,
    ) {
        println!("Ready as {}", self.cache.user().await.username);

        let user = self.cache.user().await;
        if user
            .status
            .is_none_or(|s| s.text != Some("@Roles colour".into()))
        {
            let edit = UserEdit::new().status_text("@Roles colour");
            if let Err(e) = self.http.edit_user(self.cache.user_id(), edit).await {
                dbg!(e);
            }
        }
    }

    async fn on_message(&self, message: Message) {
        if let Err(e) = self.on_message(&message).await {
            self.on_message_error(&message, e).await;
        }
    }

    async fn on_message_delete(&self, id: String, _channel_id: String) {
        self.setup_messages.write().await.remove(&id);
        self.role_messages.write().await.remove(&id);
    }

    async fn on_message_react(
        &self,
        id: String,
        channel_id: String,
        user_id: String,
        emoji_id: String,
    ) {
        if let Err(e) = self
            .on_react(&channel_id, &id, &user_id, &emoji_id, RoleReact::React)
            .await
        {
            self.on_react_error(&channel_id, &user_id, e).await;
        }
    }

    async fn on_message_unreact(
        &self,
        id: String,
        channel_id: String,
        user_id: String,
        emoji_id: String,
    ) {
        if let Err(e) = self
            .on_react(&channel_id, &id, &user_id, &emoji_id, RoleReact::Unreact)
            .await
        {
            self.on_react_error(&channel_id, &user_id, e).await;
        }
    }

    async fn on_server_member_join(&self, id: String, member: Member) {
        let user_id = &member.id.user;
        if let Err(e) = self.on_member_join(&id, user_id).await {
            self.on_member_join_error(&id, user_id, e).await;
        }
    }
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().unwrap();
    env_logger::init();

    let db = SqliteDB::new().unwrap();

    let token = std::env::var("BOT_TOKEN").expect("Missing Env Variable: BOT_TOKEN");
    let http = Http::new(&token, true);
    let ws = WebSocket::connect(&token).await;
    let cache = Cache::new();

    let bot = Bot {
        http,
        cache: cache.clone(),
        db,
        setup_messages: RwLock::new(HashMap::new()),
        role_messages: RwLock::new(HashMap::new()),
        server_handlers: RwLock::new(HashMap::new()),
    };
    let handler = Arc::new(bot);

    loop {
        let event = ws.next().await;
        cache.update(event.clone()).await;
        let h = handler.clone();
        tokio::spawn(async move {
            h.on_event(event).await;
        });
    }
}
