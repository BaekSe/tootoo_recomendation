pub mod domain;
pub mod ingest;
pub mod llm;
pub mod storage;
pub mod time;

pub mod config {
    use anyhow::Context;

    #[derive(Debug, Clone)]
    pub struct Settings {
        pub database_url: Option<String>,
        pub supabase_url: Option<String>,
        pub supabase_service_role_key: Option<String>,
        pub anthropic_api_key: Option<String>,
        pub openai_api_key: Option<String>,
        pub sentry_dsn: Option<String>,
        pub data_provider_base_url: Option<String>,
        pub data_provider_api_key: Option<String>,
    }

    impl Settings {
        pub fn from_env() -> anyhow::Result<Self> {
            Ok(Self {
                database_url: std::env::var("DATABASE_URL").ok(),
                supabase_url: std::env::var("SUPABASE_URL").ok(),
                supabase_service_role_key: std::env::var("SUPABASE_SERVICE_ROLE_KEY").ok(),
                anthropic_api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
                openai_api_key: std::env::var("OPENAI_API_KEY").ok(),
                sentry_dsn: std::env::var("SENTRY_DSN").ok(),
                data_provider_base_url: std::env::var("DATA_PROVIDER_BASE_URL").ok(),
                data_provider_api_key: std::env::var("DATA_PROVIDER_API_KEY").ok(),
            })
        }

        pub fn require_database_url(&self) -> anyhow::Result<&str> {
            self.database_url
                .as_deref()
                .context("DATABASE_URL is required")
        }

        pub fn require_anthropic_api_key(&self) -> anyhow::Result<&str> {
            self.anthropic_api_key
                .as_deref()
                .context("ANTHROPIC_API_KEY is required")
        }

        pub fn require_data_provider_base_url(&self) -> anyhow::Result<&str> {
            self.data_provider_base_url
                .as_deref()
                .context("DATA_PROVIDER_BASE_URL is required")
        }
    }
}
