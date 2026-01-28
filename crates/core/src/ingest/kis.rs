use crate::config::Settings;
use crate::ingest::types::{DailyFeatureItem, DailyFeaturesResponse};
use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate, TimeZone, Utc};
use encoding_rs::EUC_KR;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::Duration;

const PROD_BASE_URL: &str = "https://openapi.koreainvestment.com:9443";

const KOSPI_MASTER_ZIP: &str =
    "https://new.real.download.dws.co.kr/common/master/kospi_code.mst.zip";
const KOSDAQ_MASTER_ZIP: &str =
    "https://new.real.download.dws.co.kr/common/master/kosdaq_code.mst.zip";
const KONEX_MASTER_ZIP: &str =
    "https://new.real.download.dws.co.kr/common/master/konex_code.mst.zip";

#[derive(Debug)]
pub struct KisClient {
    http: reqwest::Client,
    base_url: String,
    appkey: String,
    appsecret: String,
    req_delay: Duration,
    markets: Vec<KisMarket>,

    // Cache token within a single process run to avoid repeated token issuance.
    token_cache: tokio::sync::Mutex<Option<CachedToken>>,

    // Optional persistent token cache in DB (recommended for CI runners).
    db_pool: Option<sqlx::PgPool>,
    token_env_key: String,
}

#[derive(Debug, Clone)]
struct CachedToken {
    token: KisToken,
    fetched_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KisMarket {
    Kospi,
    Kosdaq,
    Konex,
}

impl KisClient {
    pub fn from_settings_prod(_settings: &Settings) -> Result<Self> {
        let appkey = std::env::var("KIS_APPKEY").context("KIS_APPKEY is required")?;
        let appsecret = std::env::var("KIS_APPSECRET").context("KIS_APPSECRET is required")?;

        let base_url = std::env::var("KIS_BASE_URL").unwrap_or_else(|_| PROD_BASE_URL.to_string());
        let req_delay_ms = std::env::var("KIS_REQ_DELAY_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(150);

        let markets = parse_markets(std::env::var("KIS_MARKETS").ok());

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("failed to build KIS http client")?;

        Ok(Self {
            http,
            base_url,
            appkey,
            appsecret,
            req_delay: Duration::from_millis(req_delay_ms),
            markets,
            token_cache: tokio::sync::Mutex::new(None),
            db_pool: None,
            token_env_key: "prod".to_string(),
        })
    }

    pub fn with_db_pool(mut self, pool: sqlx::PgPool) -> Self {
        self.db_pool = Some(pool);
        self
    }

    pub async fn fetch_daily_features_krx(
        &self,
        as_of_date: NaiveDate,
    ) -> Result<(DailyFeaturesResponse, Value)> {
        let token = self.get_access_token_cached().await?;

        let mut items = Vec::new();
        let mut failures: usize = 0;
        let mut logged_failures: usize = 0;
        let mut universe = self.fetch_master_universe().await?;

        let max_tickers = std::env::var("KIS_MAX_TICKERS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok());
        if let Some(max) = max_tickers {
            if universe.len() > max {
                universe.truncate(max);
            }
        }

        let total = universe.len();
        let progress_every = std::env::var("KIS_PROGRESS_EVERY")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(200);

        // Fetch previous business day as start date to compute ret_1d.
        let start_date = previous_business_day(as_of_date);
        let start = start_date.format("%Y%m%d").to_string();
        let end = as_of_date.format("%Y%m%d").to_string();

        for (idx, stock) in universe.into_iter().enumerate() {
            if idx != 0 {
                tokio::time::sleep(self.req_delay).await;
            }

            match self
                .fetch_one_stock_daily_features(
                    &token, &stock, &start, &end, start_date, as_of_date,
                )
                .await
            {
                Ok(item) => items.push(item),
                Err(err) => {
                    failures += 1;
                    if logged_failures < 10 {
                        tracing::warn!(
                            idx,
                            ticker = %stock.code,
                            name = %stock.name,
                            failure_count = failures,
                            error = %err,
                            "KIS daily fetch failed; skipping stock"
                        );
                        logged_failures += 1;
                    }
                }
            }

            if progress_every != 0 {
                let n = idx + 1;
                if n == 1 || n == total || (n % progress_every == 0) {
                    tracing::info!(
                        processed = n,
                        total,
                        items = items.len(),
                        failures,
                        %as_of_date,
                        "KIS ingest progress"
                    );
                }
            }
        }

        let raw = serde_json::json!({
            "source": "kis",
            "base_url": self.base_url,
            "as_of_date": as_of_date,
            "items": items.len(),
            "failures": failures,
            "generated_at": Utc::now(),
        });

        Ok((DailyFeaturesResponse { as_of_date, items }, raw))
    }

    async fn get_access_token_cached(&self) -> Result<KisToken> {
        let mut guard = self.token_cache.lock().await;
        if let Some(cached) = guard.as_ref() {
            if !cached.token.is_expired_or_stale(cached.fetched_at) {
                return Ok(cached.token.clone());
            }
        }

        // Try persistent cache (DB) before issuing a new token.
        if let Some(pool) = self.db_pool.as_ref() {
            if let Some(tok) = load_token_from_db(pool, &self.token_env_key).await? {
                if !tok.is_expired_or_stale(chrono::Utc::now()) {
                    let now = chrono::Utc::now();
                    *guard = Some(CachedToken {
                        token: tok.clone(),
                        fetched_at: now,
                    });
                    return Ok(tok);
                }
            }
        }

        let fetched_at = chrono::Utc::now();
        let token = self.fetch_access_token().await?;
        *guard = Some(CachedToken { token: token.clone(), fetched_at });

        if let Some(pool) = self.db_pool.as_ref() {
            // Best-effort: do not fail ingestion if token persistence fails.
            if let Err(err) = save_token_to_db(pool, &self.token_env_key, &token).await {
                tracing::warn!(error = %err, "failed to persist KIS access token to DB");
            }
        }
        Ok(token)
    }

    async fn fetch_access_token(&self) -> Result<KisToken> {
        let url = format!("{}/oauth2/tokenP", self.base_url.trim_end_matches('/'));
        let req = KisTokenRequest {
            grant_type: "client_credentials",
            appkey: &self.appkey,
            appsecret: &self.appsecret,
        };

        let res = self
            .http
            .post(url)
            .header("Content-Type", "application/json")
            .header("Accept", "text/plain")
            .header("charset", "UTF-8")
            .json(&req)
            .send()
            .await
            .context("KIS token request failed")?;

        let status = res.status();
        let text = res
            .text()
            .await
            .context("failed to read KIS token response")?;
        if !status.is_success() {
            anyhow::bail!("KIS token HTTP {status}: {text}");
        }

        serde_json::from_str::<KisToken>(&text).context("failed to parse KIS token response")
    }

    async fn fetch_master_universe(&self) -> Result<Vec<KisMasterRecord>> {
        let mut out = Vec::new();
        for market in &self.markets {
            let url = match market {
                KisMarket::Kospi => KOSPI_MASTER_ZIP,
                KisMarket::Kosdaq => KOSDAQ_MASTER_ZIP,
                KisMarket::Konex => KONEX_MASTER_ZIP,
            };
            out.extend(fetch_and_parse_master_zip(&self.http, url).await?);
        }
        Ok(out)
    }

    async fn fetch_one_stock_daily_features(
        &self,
        token: &KisToken,
        stock: &KisMasterRecord,
        start: &str,
        end: &str,
        prev_date: NaiveDate,
        as_of_date: NaiveDate,
    ) -> Result<DailyFeatureItem> {
        // Daily item chart price (OHLCV + trading value + PER/PBR/EPS) endpoint.
        let url = format!(
            "{}/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice",
            self.base_url.trim_end_matches('/')
        );

        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_str(&format!("Bearer {}", token.access_token))?,
        );
        headers.insert("appkey", HeaderValue::from_str(&self.appkey)?);
        headers.insert("appsecret", HeaderValue::from_str(&self.appsecret)?);
        headers.insert("tr_id", HeaderValue::from_static("FHKST03010100"));
        headers.insert("custtype", HeaderValue::from_static("P"));
        headers.insert("tr_cont", HeaderValue::from_static(""));
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));
        headers.insert("Accept", HeaderValue::from_static("text/plain"));
        headers.insert("charset", HeaderValue::from_static("UTF-8"));

        let params = [
            ("FID_COND_MRKT_DIV_CODE", "J"),
            ("FID_INPUT_ISCD", stock.code.as_str()),
            ("FID_INPUT_DATE_1", start),
            ("FID_INPUT_DATE_2", end),
            ("FID_PERIOD_DIV_CODE", "D"),
            ("FID_ORG_ADJ_PRC", "1"),
        ];

        let max_attempts: u32 = 3;
        let mut attempt: u32 = 0;
        let body = loop {
            attempt += 1;

            let res = self
                .http
                .get(url.clone())
                .headers(headers.clone())
                .query(&params)
                .send()
                .await;

            let res = match res {
                Ok(r) => r,
                Err(err) => {
                    if attempt >= max_attempts {
                        return Err(err).context("KIS daily itemchartprice request failed");
                    }
                    let backoff = Duration::from_secs(1 << (attempt - 1));
                    tracing::warn!(
                        attempt,
                        ?backoff,
                        ticker = %stock.code,
                        error = %err,
                        "KIS daily request failed; retrying"
                    );
                    tokio::time::sleep(backoff).await;
                    continue;
                }
            };

            let status = res.status();
            let text = res
                .text()
                .await
                .context("failed to read KIS daily response")?;

            if !status.is_success() {
                let retryable = status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
                if retryable && attempt < max_attempts {
                    let backoff = Duration::from_secs(1 << (attempt - 1));
                    tracing::warn!(
                        attempt,
                        ?backoff,
                        ticker = %stock.code,
                        http_status = %status,
                        "KIS daily HTTP error; retrying"
                    );
                    tokio::time::sleep(backoff).await;
                    continue;
                }
                anyhow::bail!("KIS daily itemchartprice HTTP {status}: {text}");
            }

            match serde_json::from_str::<KisDailyItemChartPriceResponse>(&text) {
                Ok(body) => break body,
                Err(err) => {
                    if attempt >= max_attempts {
                        return Err(err)
                            .context("failed to parse KIS daily itemchartprice response");
                    }
                    let backoff = Duration::from_secs(1 << (attempt - 1));
                    tracing::warn!(
                        attempt,
                        ?backoff,
                        ticker = %stock.code,
                        error = %err,
                        "KIS daily response parse failed; retrying"
                    );
                    tokio::time::sleep(backoff).await;
                    continue;
                }
            }
        };

        // Find prev and as-of records.
        let prev_ymd = prev_date.format("%Y%m%d").to_string();
        let asof_ymd = as_of_date.format("%Y%m%d").to_string();

        let mut prev_close: Option<f64> = None;
        let mut asof: Option<&KisDailyBar> = None;
        for bar in &body.output2 {
            if bar.stck_bsop_date == prev_ymd {
                prev_close = parse_num(&bar.stck_clpr);
            }
            if bar.stck_bsop_date == asof_ymd {
                asof = Some(bar);
            }
        }

        let asof = asof.context("missing as-of bar in KIS response")?;

        let close = parse_num(&asof.stck_clpr).context("missing close")?;
        let trading_value = parse_num(&asof.acml_tr_pbmn);
        let volume = parse_num(&asof.acml_vol);

        let ret_1d = prev_close.map(|p| (close / p) - 1.0);

        let mut features = BTreeMap::<String, f64>::new();
        if let Some(v) = ret_1d {
            features.insert("ret_1d".to_string(), v);
        }
        if let Some(v) = trading_value {
            features.insert("trading_value".to_string(), v);
        }
        if let Some(v) = volume {
            features.insert("volume".to_string(), v);
        }

        if let Some(v) = parse_num(&asof.per) {
            features.insert("per".to_string(), v);
        }
        if let Some(v) = parse_num(&asof.pbr) {
            features.insert("pbr".to_string(), v);
        }
        if let Some(v) = parse_num(&asof.eps) {
            features.insert("eps".to_string(), v);
        }

        Ok(DailyFeatureItem {
            ticker: format!("KRX:{}", stock.code),
            name: stock.name.clone(),
            trading_value,
            features,
        })
    }
}

#[derive(Debug, Serialize)]
struct KisTokenRequest<'a> {
    grant_type: &'a str,
    appkey: &'a str,
    appsecret: &'a str,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KisToken {
    pub access_token: String,
    #[serde(default)]
    pub access_token_token_expired: String,

    #[serde(default)]
    pub expires_in: u64,
}

impl KisToken {
    fn is_expired_or_stale(&self, fetched_at: chrono::DateTime<chrono::Utc>) -> bool {
        // Prefer the server-provided absolute expiry when available.
        if let Some(exp) = parse_kis_expiry_utc(&self.access_token_token_expired) {
            // Refresh a bit early to avoid edge races.
            return chrono::Utc::now() + chrono::Duration::minutes(2) >= exp;
        }

        // Fallback to relative expires_in.
        if self.expires_in > 0 {
            let exp = fetched_at + chrono::Duration::seconds(self.expires_in as i64);
            return chrono::Utc::now() + chrono::Duration::minutes(2) >= exp;
        }

        // Conservative default: treat unknown expiry as stale.
        true
    }
}

async fn load_token_from_db(pool: &sqlx::PgPool, env: &str) -> Result<Option<KisToken>> {
    let row = sqlx::query_as::<_, (String, Option<String>, Option<i64>)>(
        "SELECT access_token, access_token_token_expired, expires_in \
         FROM kis_access_tokens \
         WHERE env = $1",
    )
    .persistent(false)
    .bind(env)
    .fetch_optional(pool)
    .await;

    let Some((access_token, token_expired, expires_in)) = row.ok().flatten() else {
        return Ok(None);
    };

    Ok(Some(KisToken {
        access_token,
        access_token_token_expired: token_expired.unwrap_or_default(),
        expires_in: expires_in.unwrap_or(0).max(0) as u64,
    }))
}

async fn save_token_to_db(pool: &sqlx::PgPool, env: &str, tok: &KisToken) -> Result<()> {
    sqlx::query(
        "INSERT INTO kis_access_tokens (env, access_token, access_token_token_expired, expires_in, issued_at, updated_at) \
         VALUES ($1, $2, $3, $4, now(), now()) \
         ON CONFLICT (env) DO UPDATE SET \
           access_token = EXCLUDED.access_token, \
           access_token_token_expired = EXCLUDED.access_token_token_expired, \
           expires_in = EXCLUDED.expires_in, \
           updated_at = now()",
    )
    .persistent(false)
    .bind(env)
    .bind(&tok.access_token)
    .bind(&tok.access_token_token_expired)
    .bind(tok.expires_in as i64)
    .execute(pool)
    .await
    .context("upsert kis_access_tokens failed")?;
    Ok(())
}

fn parse_kis_expiry_utc(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }

    // Observed format: "YYYY-MM-DD HH:MM:SS" (KST). Convert to UTC.
    let naive = chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M:%S").ok()?;
    // KST is UTC+9.
    let kst = chrono::FixedOffset::east_opt(9 * 3600)?;
    let dt = kst.from_local_datetime(&naive).single()?;
    Some(dt.with_timezone(&chrono::Utc))
}

#[cfg(test)]
mod token_tests {
    use super::*;

    #[test]
    fn parses_token_expiry_fields() {
        let s = r#"{
            "access_token": "secret",
            "token_type": "Bearer",
            "expires_in": 86400,
            "access_token_token_expired": "2026-01-30 05:00:44"
        }"#;

        let tok: KisToken = serde_json::from_str(s).unwrap();
        assert_eq!(tok.expires_in, 86400);
        assert_eq!(tok.access_token, "secret");
        assert_eq!(tok.access_token_token_expired, "2026-01-30 05:00:44");
        let dt = parse_kis_expiry_utc(&tok.access_token_token_expired).unwrap();
        assert_eq!(dt.to_rfc3339(), "2026-01-29T20:00:44+00:00");
    }
}

#[derive(Debug, Clone, Deserialize)]
struct KisDailyItemChartPriceResponse {
    #[serde(default)]
    output2: Vec<KisDailyBar>,
}

#[derive(Debug, Clone, Deserialize)]
struct KisDailyBar {
    #[serde(default)]
    stck_bsop_date: String,
    #[serde(default)]
    stck_clpr: String,
    #[serde(default)]
    acml_tr_pbmn: String,
    #[serde(default)]
    acml_vol: String,
    #[serde(default)]
    per: String,
    #[serde(default)]
    pbr: String,
    #[serde(default)]
    eps: String,
}

#[derive(Debug, Clone)]
struct KisMasterRecord {
    code: String,
    name: String,
}

fn parse_markets(v: Option<String>) -> Vec<KisMarket> {
    let Some(v) = v else {
        return vec![KisMarket::Kospi, KisMarket::Kosdaq];
    };
    let mut out = Vec::new();
    for part in v.split(',') {
        match part.trim().to_ascii_uppercase().as_str() {
            "KOSPI" => out.push(KisMarket::Kospi),
            "KOSDAQ" => out.push(KisMarket::Kosdaq),
            "KONEX" => out.push(KisMarket::Konex),
            _ => {}
        }
    }
    if out.is_empty() {
        out.push(KisMarket::Kospi);
        out.push(KisMarket::Kosdaq);
    }
    out
}

fn previous_business_day(d: NaiveDate) -> NaiveDate {
    // Basic weekend rollback. Holiday calendar is handled elsewhere in the worker; for ingestion
    // we keep this minimal.
    let mut cur = d - chrono::Duration::days(1);
    while matches!(cur.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun) {
        cur = cur - chrono::Duration::days(1);
    }
    cur
}

fn parse_num(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<f64>().ok()
}

async fn fetch_and_parse_master_zip(
    http: &reqwest::Client,
    url: &str,
) -> Result<Vec<KisMasterRecord>> {
    let res = http
        .get(url)
        .send()
        .await
        .context("master zip download failed")?;
    let status = res.status();
    let bytes = res.bytes().await.context("read master zip bytes failed")?;
    if !status.is_success() {
        anyhow::bail!("master zip HTTP {status}");
    }

    let bytes_vec = bytes.to_vec();
    let records = tokio::task::spawn_blocking(move || unzip_and_parse_master(&bytes_vec))
        .await
        .context("join unzip task failed")??;
    Ok(records)
}

fn unzip_and_parse_master(zip_bytes: &[u8]) -> Result<Vec<KisMasterRecord>> {
    use std::io::{Cursor, Read};

    let reader = Cursor::new(zip_bytes);
    let mut zip = zip::ZipArchive::new(reader).context("open zip archive failed")?;
    anyhow::ensure!(zip.len() >= 1, "zip has no entries");

    let mut mst_idx: Option<usize> = None;
    for i in 0..zip.len() {
        let name = {
            let f = zip.by_index(i).context("open zip entry failed")?;
            f.name().to_string()
        };
        if name.to_ascii_lowercase().ends_with(".mst") {
            mst_idx = Some(i);
            break;
        }
    }
    let idx = mst_idx.unwrap_or(0);

    let mut file = zip.by_index(idx).context("open zip entry failed")?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .context("read zip entry failed")?;

    parse_master_lines(&buf)
}

fn parse_master_lines(buf: &[u8]) -> Result<Vec<KisMasterRecord>> {
    let mut out = Vec::new();
    for line in buf.split(|b| *b == b'\n') {
        let line = if line.last().copied() == Some(b'\r') {
            &line[..line.len().saturating_sub(1)]
        } else {
            line
        };
        if line.len() < 6 {
            continue;
        }

        let code_bytes = &line[0..6];
        if !code_bytes.iter().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let code = std::str::from_utf8(code_bytes).unwrap_or("").to_string();

        // After the 6-digit code, expect spaces, then ISIN, then name, then a market marker (ST...).
        let mut i = 6;
        while i < line.len() && line[i].is_ascii_whitespace() {
            i += 1;
        }

        // ISIN is fixed-width (12 bytes) in the master file and may not be separated by spaces.
        let isin_start = i;
        let name_start = if line.len() >= isin_start + 12 {
            isin_start + 12
        } else {
            // Fallback: read token until whitespace.
            while i < line.len() && !line[i].is_ascii_whitespace() {
                i += 1;
            }
            i
        };

        if name_start >= line.len() {
            continue;
        }

        let after_name = &line[name_start..];
        let st_pos = find_st_marker(after_name).unwrap_or(after_name.len());
        let name = decode_euc_kr_trim(&after_name[..st_pos]);
        if name.is_empty() {
            continue;
        }

        out.push(KisMasterRecord { code, name });
    }
    Ok(out)
}

fn find_st_marker(bytes: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'S' && bytes[i + 1] == b'T' {
            // Heuristic: require preceding space.
            if i == 0 || bytes[i - 1].is_ascii_whitespace() {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn decode_euc_kr_trim(bytes: &[u8]) -> String {
    // Trim ASCII whitespace and NULs.
    let mut start = 0;
    let mut end = bytes.len();
    while start < end && (bytes[start].is_ascii_whitespace() || bytes[start] == 0) {
        start += 1;
    }
    while end > start && (bytes[end - 1].is_ascii_whitespace() || bytes[end - 1] == 0) {
        end -= 1;
    }
    let slice = &bytes[start..end];

    let (cow, _, _) = EUC_KR.decode(slice);
    cow.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_master_line_with_code_prefix() {
        // Minimal synthetic line similar to: "005930   KR7005930003...<name>...ST..."
        let mut line = b"005930   KR7005930003".to_vec();
        let (name_bytes, _, _) = EUC_KR.encode("삼성전자");
        line.extend_from_slice(&name_bytes);
        line.extend_from_slice(b"                ST1002700\n");

        let parsed = parse_master_lines(&line).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].code, "005930");
    }
}
