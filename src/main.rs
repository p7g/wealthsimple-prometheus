use chrono::{DateTime, Utc};
use lazy_static::lazy_static;
use prometheus::{self, register_gauge_vec, Encoder, GaugeVec, TextEncoder};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use tiny_http::{Response, Server};

macro_rules! api {
    ($path:expr) => {
        format!("https://api.production.wealthsimple.com/v1/{}", $path)
    };
}

lazy_static! {
    // TODO: tag with other useful things like account status, currency, etc.
    static ref DEPOSITED: GaugeVec = register_gauge_vec!(
        "wealthsimple_deposited",
        "the total amount deposited",
        &["account_id", "account_type", "account_name"]
    )
    .unwrap();
    static ref WITHDRAWN: GaugeVec = register_gauge_vec!(
        "wealthsimple_withdrawn",
        "the total amount withdrawn",
        &["account_id", "account_type", "account_name"]
    )
    .unwrap();
    static ref NET_LIQUIDATION: GaugeVec = register_gauge_vec!(
        "wealthsimple_net_liquidation",
        "the value of the account if it were to be liquidated",
        &["account_id", "account_type", "account_name"]
    ).unwrap();
    static ref GROSS_POSITION: GaugeVec = register_gauge_vec!(
        "wealthsimple_gross_position",
        "sum of all positions in the account",
        &["account_id", "account_type", "account_name"]
    ).unwrap();
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
enum Currency {
    Cad,
    Usd,
    Eur,
    Gbp,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Open,
    Closed,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum OwnershipType {
    Primary,
    Secondary,
}

#[derive(Debug, Serialize, Deserialize)]
struct Owner<'a> {
    client_id: &'a str,
    ownership_type: OwnershipType,
    account_nickname: Option<&'a str>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Amount<'a> {
    amount: &'a str,
    currency: Currency,
}

#[derive(Debug, Serialize, Deserialize)]
struct Account<'a> {
    object: &'a str,
    id: &'a str,
    #[serde(rename = "type")]
    type_: &'a str,
    nickname: Option<&'a str>,
    base_currency: Currency,
    status: Status,
    owners: Vec<Owner<'a>>,
    net_liquidation: Amount<'a>,
    gross_position: Amount<'a>,
    total_deposits: Amount<'a>,
    total_withdrawals: Amount<'a>,
    withdrawn_earnings: Amount<'a>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct AccountsResponse<'a> {
    object: &'a str,
    offset: i64,
    total_count: i64,
    #[serde(borrow)]
    results: Vec<Account<'a>>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let username = rprompt::prompt_reply_stdout("Email: ")?;
    let password = rpassword::prompt_password_stdout("Password: ")?;
    let id = uuid::Uuid::new_v4().to_simple().to_string();
    let mut otp_claim = None;

    let mut token = login(&id, &username, &password, &mut otp_claim)?;

    std::thread::spawn(|| {
        let server = Server::http("0.0.0.0:8080").unwrap();

        for request in server.incoming_requests() {
            if request.url() != "/metrics" {
                if let Err(e) = request.respond(Response::empty(404)) {
                    eprintln!("Failed to respond to request: {}", e);
                }
                continue;
            }

            let mut buffer = Vec::new();
            let encoder = TextEncoder::new();

            let metrics = prometheus::gather();
            if let Err(e) = encoder.encode(&metrics, &mut buffer) {
                eprintln!("Failed to encode metrics data: {}", e);
                continue;
            }

            let output = String::from_utf8(buffer).unwrap();
            if let Err(e) = request.respond(Response::from_string(output)) {
                eprintln!("Failed to send metrics data: {}", e);
            }
        }
    });

    loop {
        let resp = minreq::get(api!("accounts"))
            .with_header("Authorization", &token)
            .with_header("Accept", "*/*")
            .with_header("User-Agent", "curl/7.64.1")
            .send()?;

        if resp.status_code == 401 {
            println!(
                "got 401, need to log in again: {}",
                std::str::from_utf8(resp.as_bytes())?
            );
            token = login(&id, &username, &password, &mut otp_claim)?;
            continue;
        } else if resp.status_code != 200 {
            return Err(
                format!("Request failed: {}", std::str::from_utf8(resp.as_bytes())?).into(),
            );
        }

        let accounts: AccountsResponse = resp.json()?;

        for account in accounts.results {
            let label_values = &[account.id, account.type_, account.nickname.unwrap_or("")];

            if let Ok(amount) = f64::from_str(account.total_deposits.amount) {
                DEPOSITED.with_label_values(label_values).set(amount);
            }

            if let Ok(amount) = f64::from_str(account.total_withdrawals.amount) {
                WITHDRAWN.with_label_values(label_values).set(amount);
            }

            if let Ok(amount) = f64::from_str(account.net_liquidation.amount) {
                NET_LIQUIDATION.with_label_values(label_values).set(amount);
            }

            if let Ok(amount) = f64::from_str(account.gross_position.amount) {
                GROSS_POSITION.with_label_values(label_values).set(amount);
            }
        }

        std::thread::sleep(std::time::Duration::from_secs(300));
    }
}

#[derive(Deserialize)]
struct LoginResponse<'a> {
    access_token: &'a str,
}

/**
* Request an auth token, prompting the user for their username, password, and
* 2FA code if applicable
*/
fn login(
    id: &str,
    username: &str,
    password: &str,
    otp_claim: &mut Option<String>,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut payload = HashMap::new();

    payload.insert("username", username);
    payload.insert("password", password);
    payload.insert("scope", "invest.read mfda.read mercer.read trade.read");
    payload.insert("grant_type", "password");
    payload.insert(
        "client_id",
        "4da53ac2b03225bed1550eba8e4611e086c7b905a3855e6ed12ea08c246758fa",
    );

    let mut req = minreq::post(api!("oauth/token"))
        .with_header("Accept", "application/json")
        .with_header("User-Agent", "curl/7.64.1");

    if let Some(claim) = otp_claim {
        req = req.with_header("x-wealthsimple-otp-claim", &*claim);
    }

    let resp = req.with_json(&payload)?.send()?;

    match resp.status_code {
        401 if resp
            .headers
            .get("x-wealthsimple-otp")
            .map(|s| s == "required; method=app")
            .unwrap_or(false) =>
        {
            let otp = rprompt::prompt_reply_stdout("2FA code: ")?;
            let resp = minreq::post(api!("oauth/token"))
                .with_header("Accept", "application/json")
                .with_header("User-Agent", "curl/7.64.1")
                .with_header("x-wealthsimple-otp", format!("{};remember=true", otp))
                .with_header("x-ws-device-id", id)
                .with_json(&payload)?
                .send()?;

            if resp.status_code == 200 {
                otp_claim.replace(resp.headers["x-wealthsimple-otp-claim"].clone());
                let body: LoginResponse = resp.json()?;
                Ok(format!("Bearer {}", body.access_token))
            } else {
                Err(format!(
                    "Failed to log in after 2fa: {}",
                    std::str::from_utf8(resp.as_bytes())?
                )
                .into())
            }
        }
        200 => {
            let body: LoginResponse = resp.json()?;
            Ok(format!("Bearer {}", body.access_token))
        }
        _ => Err(format!(
            "Failed to log in: {:#?} {}",
            resp.headers,
            std::str::from_utf8(resp.as_bytes())?,
        )
        .into()),
    }
}
