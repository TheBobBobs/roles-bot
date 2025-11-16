use volty::prelude::*;

use crate::{error::Error, reaction::RoleAction, Bot};

impl Bot {
    pub async fn on_member_join(&self, server_id: &str, user_id: &str) -> Result<(), Error> {
        let Some(settings) = self.db.get_settings(server_id).await else {
            return Ok(());
        };
        if settings.auto_roles.is_empty() {
            return Ok(());
        };
        let user = self.cache.fetch_user(&self.http, user_id).await?;
        if user.bot.is_some() {
            return Ok(());
        }

        let Some(server) = self.cache.get_server(server_id).await else {
            return Ok(());
        };
        let mut roles = settings.auto_roles.clone();
        roles.retain(|role_id| server.roles.contains_key(role_id));
        if roles.len() != settings.auto_roles.len() {
            let mut settings = settings;
            settings.auto_roles = roles.clone();
            self.db.save_settings(settings).await?;
        }

        let my_id = self.cache.user_id();
        self.check_server_perms(server_id, my_id, &[Permission::AssignRoles])
            .await?;
        self.check_above_roles(server_id, my_id, roles.iter().map(|s| s.as_str()))
            .await?;

        println!("AutoRole: {server_id}, {user_id}, {:?}", &roles);
        if !roles.is_empty() {
            self.queue_edit(
                server_id,
                user_id.to_string(),
                RoleAction {
                    give: roles,
                    remove: vec![],
                },
            )
            .await;
        }
        Ok(())
    }

    pub async fn on_member_join_error(&self, server_id: &str, user_id: &str, e: Error) {
        dbg!(server_id, user_id, e);
    }
}
