use std::collections::HashMap;

use rusqlite::Connection;
use tokio::sync::{Mutex, RwLock};

#[derive(Clone, Debug)]
pub struct ServerSettings {
    pub id: String,
    pub auto_roles: Vec<String>,
}

pub struct SqliteDB {
    pub conn: Mutex<Connection>,
    servers: RwLock<HashMap<String, ServerSettings>>,
}

impl SqliteDB {
    pub fn new() -> Result<Self, rusqlite::Error> {
        let conn = Connection::open("roles.sqlite")?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS autoroles (
                server_id TEXT NOT NULL,
                role_id TEXT NOT NULL,
                PRIMARY KEY (server_id, role_id)
            )",
            (),
        )?;

        let mut servers = HashMap::new();
        let mut stmt = conn.prepare("SELECT server_id, role_id FROM autoroles")?;
        let rows = stmt.query_map((), |r| Ok((r.get(0)?, r.get(1)?)))?;
        for row in rows {
            let (server_id, role_id): (String, String) = row?;
            let settings = servers
                .entry(server_id.clone())
                .or_insert_with(|| ServerSettings {
                    id: server_id,
                    auto_roles: Vec::new(),
                });
            settings.auto_roles.push(role_id);
        }
        drop(stmt);

        let conn = Mutex::new(conn);
        let servers = RwLock::new(servers);
        Ok(Self { conn, servers })
    }

    pub async fn get_settings(&self, id: &str) -> Option<ServerSettings> {
        self.servers.read().await.get(id).cloned()
    }

    pub async fn save_settings(&self, server: ServerSettings) -> Result<(), rusqlite::Error> {
        {
            let mut conn = self.conn.lock().await;
            let txn = conn.transaction()?;
            txn.execute("DELETE FROM autoroles WHERE server_id = ?", (&server.id,))?;
            let mut stmt =
                txn.prepare("INSERT INTO autoroles (server_id, role_id) VALUES (?, ?)")?;
            for role_id in &server.auto_roles {
                stmt.execute((&server.id, role_id))?;
            }
            drop(stmt);
            txn.commit()?;
        }
        self.servers.write().await.insert(server.id.clone(), server);
        Ok(())
    }
}
