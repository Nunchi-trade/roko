//! auth command handlers.
#![allow(unused_imports)]

use crate::*;

pub(crate) async fn cmd_login(
    url: &str,
    api_key_mode: bool,
    check: bool,
    dashboard_url: &str,
) -> Result<i32> {
    use roko_cli::credentials;

    let url = url.trim_end_matches('/');

    if api_key_mode {
        if check {
            // Non-interactive: validate stored credential only.
            match credentials::load_credential()? {
                Some(cred) => match validate_credential(&cred.url, &cred.token).await {
                    Ok(true) => {
                        println!("authenticated to {} (stored credential valid)", cred.url);
                        return Ok(EXIT_SUCCESS);
                    }
                    Ok(false) => {
                        eprintln!("stored credential for {} is no longer valid", cred.url);
                        return Ok(EXIT_FAILURE);
                    }
                    Err(e) => {
                        eprintln!("could not connect to {}: {e}", cred.url);
                        return Ok(EXIT_FAILURE);
                    }
                },
                None => {
                    eprintln!("no stored credential found; run `roko login` to authenticate");
                    return Ok(EXIT_FAILURE);
                }
            }
        }

        // Interactive API key entry.
        if !std::io::stdin().is_terminal() {
            anyhow::bail!(
                "stdin is not a terminal; use --api-key --check for non-interactive mode"
            );
        }

        print!("Enter API key for {url}: ");
        std::io::Write::flush(&mut std::io::stdout())?;

        let key = read_password_from_terminal().unwrap_or_else(|_| {
            let mut buf = String::new();
            let _ = std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut buf);
            buf
        });
        let key = key.trim().to_string();

        if key.is_empty() {
            anyhow::bail!("empty API key");
        }

        match validate_credential(url, &key).await {
            Ok(true) => {
                let cred = credentials::Credential {
                    url: url.to_string(),
                    token: key,
                    method: "api_key".into(),
                    stored_at: chrono::Utc::now().to_rfc3339(),
                    privy_user_id: None,
                    email: None,
                    wallet_address: None,
                    login_method: None,
                };
                credentials::store_credential(&cred)?;
                println!("authenticated to {url}");
                println!(
                    "credentials stored in {}",
                    credentials::credentials_path().display()
                );
                return Ok(EXIT_SUCCESS);
            }
            Ok(false) => {
                eprintln!("invalid API key (server returned 401)");
                return Ok(EXIT_FAILURE);
            }
            Err(e) => {
                eprintln!("could not connect to {url}: {e}");
                return Ok(EXIT_FAILURE);
            }
        }
    }

    // Default: browser-based Privy auth flow.
    cmd_login_browser(url, dashboard_url).await
}

/// Browser-based login: start a localhost callback server, open the
/// dashboard's `/cli/auth` page, wait for Privy credentials.
pub(crate) async fn cmd_login_browser(url: &str, dashboard_url: &str) -> Result<i32> {
    use roko_cli::credentials;
    use tokio::sync::oneshot;

    // Callback payload from the dashboard's CliAuth page.
    #[derive(serde::Deserialize)]
    struct CallbackPayload {
        access_token: String,
        privy_user_id: String,
        email: Option<String>,
        wallet_address: Option<String>,
        #[serde(default)]
        login_method: Option<String>,
    }

    let (tx, rx) = oneshot::channel::<CallbackPayload>();
    let tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(tx)));

    // Build the callback server.
    let tx_for_handler = std::sync::Arc::clone(&tx);
    let app = axum::Router::new()
        .route(
            "/callback",
            axum::routing::options(|| async {
                axum::http::Response::builder()
                    .status(204)
                    .header("Access-Control-Allow-Origin", "*")
                    .header("Access-Control-Allow-Methods", "POST, OPTIONS")
                    .header("Access-Control-Allow-Headers", "Content-Type")
                    .body(axum::body::Body::empty())
                    .expect("invariant: cors preflight response builds")
            }),
        )
        .route(
            "/callback",
            axum::routing::post(move |axum::Json(payload): axum::Json<CallbackPayload>| {
                let tx = std::sync::Arc::clone(&tx_for_handler);
                async move {
                    if let Some(sender) = tx.lock().await.take() {
                        let _ = sender.send(payload);
                    }
                    axum::http::Response::builder()
                        .status(200)
                        .header("Access-Control-Allow-Origin", "*")
                        .body(axum::body::Body::from(r#"{"ok":true}"#))
                        .expect("invariant: callback response builds")
                }
            }),
        );

    // Bind to a random port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    // Spawn the server.
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    // Open the browser.
    let dashboard_url = dashboard_url.trim_end_matches('/');
    let auth_url = format!("{dashboard_url}/cli/auth?port={port}");
    println!("Opening browser to authenticate...");
    if webbrowser::open(&auth_url).is_err() {
        println!("Could not open browser automatically.");
        println!("Please open this URL manually:\n  {auth_url}");
    }

    print!("Waiting for login... ");
    std::io::Write::flush(&mut std::io::stdout())?;

    // Wait for the callback with a 5-minute timeout.
    let result = tokio::time::timeout(Duration::from_secs(300), rx).await;
    server_handle.abort();

    match result {
        Ok(Ok(payload)) => {
            println!("done");
            let display_name = payload.email.as_deref().unwrap_or(&payload.privy_user_id);
            println!("\nAuthenticated as {display_name}");

            let cred = credentials::Credential {
                url: url.to_string(),
                token: payload.access_token,
                method: "privy".into(),
                stored_at: chrono::Utc::now().to_rfc3339(),
                privy_user_id: Some(payload.privy_user_id),
                email: payload.email,
                wallet_address: payload.wallet_address,
                login_method: payload.login_method,
            };
            credentials::store_credential(&cred)?;
            println!(
                "Credentials saved to {}",
                credentials::credentials_path().display()
            );
            Ok(EXIT_SUCCESS)
        }
        Ok(Err(_)) => {
            println!("failed");
            anyhow::bail!("callback channel closed unexpectedly");
        }
        Err(_) => {
            println!("timed out");
            anyhow::bail!("no response within 5 minutes; run `roko login` to try again");
        }
    }
}

/// Read a password from the terminal without echoing characters.
///
/// Uses raw terminal mode via crossterm (already a dependency) to suppress
/// echo while the user types. Falls back to plain read on error.
pub(crate) fn read_password_from_terminal() -> Result<String> {
    use crossterm::terminal;
    use std::io::Read as _;

    terminal::enable_raw_mode()?;
    let mut buf = Vec::new();
    let stdin = std::io::stdin();
    let mut handle = stdin.lock();

    loop {
        let mut byte = [0u8; 1];
        handle.read_exact(&mut byte)?;
        match byte[0] {
            b'\n' | b'\r' => break,
            // Backspace / DEL
            0x7f | 0x08 => {
                buf.pop();
            }
            // Ctrl-C
            3 => {
                terminal::disable_raw_mode()?;
                println!();
                anyhow::bail!("interrupted");
            }
            b => buf.push(b),
        }
    }

    terminal::disable_raw_mode()?;
    println!(); // newline after hidden input
    Ok(String::from_utf8_lossy(&buf).to_string())
}

/// Validate an API key against a roko-serve instance.
///
/// Calls `GET {url}/api/health` with the `X-Api-Key` header.
/// Returns `Ok(true)` if the server responds 200, `Ok(false)` if 401/403,
/// and `Err` on connection failure.
pub(crate) async fn validate_credential(url: &str, token: &str) -> Result<bool> {
    let client = reqwest::Client::builder()
        .user_agent("roko-cli/0.1")
        .timeout(Duration::from_secs(10))
        .build()?;

    let resp = client
        .get(format!("{url}/api/health"))
        .header("X-Api-Key", token)
        .send()
        .await?;

    match resp.status().as_u16() {
        200..=299 => Ok(true),
        401 | 403 => Ok(false),
        other => anyhow::bail!("unexpected status {other}"),
    }
}

pub(crate) fn cmd_logout() -> Result<i32> {
    use roko_cli::credentials;

    let path = credentials::credentials_path();
    if path.exists() {
        credentials::clear_credential()?;
        println!("credentials removed from {}", path.display());
    } else {
        println!("no stored credentials found");
    }
    Ok(EXIT_SUCCESS)
}

pub(crate) async fn cmd_whoami() -> Result<i32> {
    use roko_cli::credentials;

    match credentials::load_credential()? {
        Some(cred) => {
            println!("server:     {}", cred.url);
            println!("method:     {}", cred.method);
            println!("stored at:  {}", cred.stored_at);

            // Show Privy-specific fields.
            if let Some(ref user_id) = cred.privy_user_id {
                println!("user:       {user_id}");
            }
            if let Some(ref email) = cred.email {
                println!("email:      {email}");
            }
            if let Some(ref wallet) = cred.wallet_address {
                println!("wallet:     {wallet}");
            }
            if let Some(ref method) = cred.login_method {
                println!("via:        {method}");
            }

            // Mask the token for display: show first 8 chars + "..."
            let masked = if cred.token.len() > 8 {
                format!("{}...", &cred.token[..8])
            } else {
                "****".to_string()
            };
            println!("token:      {masked}");

            // Validate the credential is still good.
            if cred.method == "api_key" {
                print!("status:     ");
                std::io::Write::flush(&mut std::io::stdout())?;
                match validate_credential(&cred.url, &cred.token).await {
                    Ok(true) => println!("valid"),
                    Ok(false) => println!("invalid (server returned 401)"),
                    Err(e) => println!("unreachable ({e})"),
                }
            }
            Ok(EXIT_SUCCESS)
        }
        None => {
            println!("not logged in");
            println!("run `roko login` to authenticate");
            Ok(EXIT_SUCCESS)
        }
    }
}
