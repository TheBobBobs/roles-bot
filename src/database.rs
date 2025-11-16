use std::collections::HashMap;

use futures::TryStreamExt;
use mongodb::{
    bson::{doc, to_document},
    options::ClientOptions,
    Client, Collection,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

#[derive(Clone, Debug)]
pub struct ServerSettings {
    pub id: String,
    pub auto_roles: Vec<String>,
}

impl From<ServerSettingsDoc> for ServerSettings {
    fn from(value: ServerSettingsDoc) -> Self {
        Self {
            id: value._id,
            auto_roles: value.auto_roles,
        }
    }
}

#[derive(Deserialize, Serialize)]
struct ServerSettingsDoc {
    _id: String,
    auto_roles: Vec<String>,
}

impl From<ServerSettings> for ServerSettingsDoc {
    fn from(value: ServerSettings) -> Self {
        Self {
            _id: value.id,
            auto_roles: value.auto_roles,
        }
    }
}

pub struct DB {
    server_col: Collection<ServerSettingsDoc>,
    servers: RwLock<HashMap<String, ServerSettings>>,
}

impl DB {
    pub async fn new(
        uri: &str,
        db_name: &str,
        server_col: &str,
    ) -> Result<DB, mongodb::error::Error> {
        let mut options = ClientOptions::parse(uri).await?;
        options.app_name = Some("RolesBot".to_string());
        let client = Client::with_options(options)?;
        let db = client.database(db_name);
        let server_col: Collection<ServerSettingsDoc> = db.collection(server_col);

        let mut servers = HashMap::new();

        let mut cursor = server_col.find(doc! {}).await?;
        while let Some(server_doc) = cursor.try_next().await? {
            let server: ServerSettings = server_doc.into();
            servers.insert(server.id.clone(), server);
        }

        Ok(Self {
            server_col,
            servers: RwLock::new(servers),
        })
    }

    pub async fn get_settings(&self, id: &str) -> Option<ServerSettings> {
        let servers = self.servers.read().await;
        servers.get(id).cloned()
    }

    pub async fn save_settings(&self, server: ServerSettings) -> Result<(), mongodb::error::Error> {
        let mut servers = self.servers.write().await;
        let server_doc: ServerSettingsDoc = server.clone().into();
        let filter = doc! {"_id": &server_doc._id};
        let mut update = doc! {"$set": to_document(&server_doc).unwrap()};
        update.remove("_id");
        self.server_col
            .update_one(filter, update)
            .upsert(true)
            .await?;
        servers.insert(server.id.clone(), server);
        Ok(())
    }
}
