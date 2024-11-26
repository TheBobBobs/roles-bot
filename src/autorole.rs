use crate::{error::Error, reaction::RoleAction, Bot};

impl Bot {
    pub async fn on_member_join(&self, server_id: &str, user_id: &str) -> Result<(), Error> {
        let Some(settings) = self.db.get_settings(server_id).await else {
            return Ok(());
        };
        let Some(auto_role) = settings.auto_role else {
            return Ok(());
        };
        let user = self.cache.fetch_user(&self.http, user_id).await?;
        if user.bot.is_some() {
            return Ok(());
        }

        println!("AutoRole: {server_id}, {user_id}, {auto_role}");
        self.queue_edit(
            server_id,
            user_id.to_string(),
            RoleAction {
                give: vec![auto_role],
                remove: vec![],
            },
        )
        .await;
        Ok(())
    }

    pub async fn on_member_join_error(&self, server_id: &str, user_id: &str, e: Error) {
        dbg!(server_id, user_id, e);
    }
}
