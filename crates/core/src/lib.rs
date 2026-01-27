pub mod domain;
pub mod llm;
pub mod storage;

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
    }
}
