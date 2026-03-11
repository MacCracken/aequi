pub struct ServerState {
    pub db: aequi_storage::DbPool,
    pub api_key: Option<String>,
}
