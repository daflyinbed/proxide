use crate::error::{WebError, WebResult};
use crate::middleware::auth::hash_token;
use crate::state::{AppState, LoginSession};
use axum::extract::{Path, Query, State};
use axum::response::{Html, IntoResponse, Response};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct CasCallbackQuery {
    pub ticket: Option<String>,
}

pub async fn cas_callback(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<CasCallbackQuery>,
) -> WebResult<Response> {
    let session = state
        .login_sessions
        .get(&session_id)
        .ok_or_else(|| WebError::NotFound("Session not found".to_string()))?;

    if session.expired_at < chrono::Utc::now().naive_utc() {
        state.login_sessions.remove(&session_id);
        return Err(WebError::Unauthorized("Session expired".to_string()));
    }

    let ticket = query
        .ticket
        .ok_or_else(|| WebError::BadRequest("Missing ticket parameter".to_string()))?;

    let root_url = &state.config.server.root_url;
    let cas_url = &state.config.auth.cas_url;
    let service_url = format!("{root_url}/api/auth/cas/callback/session/{session_id}");
    let validate_url = format!(
        "{cas_url}/cas/serviceValidate?service={service_url}&ticket={ticket}"
    );

    let cas_response = state
        .http
        .get(&validate_url)
        .send()
        .await
        .map_err(|e| WebError::CustomApiError(anyhow::anyhow!("CAS validate request failed: {e}")))?
        .text()
        .await
        .map_err(|e| WebError::CustomApiError(anyhow::anyhow!("CAS validate read failed: {e}")))?;

    let username = parse_cas_response(&cas_response)?;

    let user_id = state
        .repo
        .upsert_user(&username, None, "")
        .await
        .map_err(WebError::CustomApiError)?;

    let raw_token = uuid::Uuid::new_v4().to_string();
    let token_key = hash_token(&raw_token);

    state
        .repo
        .create_token(&token_key, "web-login", user_id, false, None, None)
        .await
        .map_err(WebError::CustomApiError)?;

    state
        .login_sessions
        .insert(
            session_id.clone(),
            LoginSession {
                token: Some(raw_token),
                user_id: Some(user_id),
                expired_at: session.expired_at,
            },
        );

    log::info!(action = "cas_login"; "user={}", username);

    Ok(Html(success_html()).into_response())
}

fn parse_cas_response(xml: &str) -> WebResult<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut in_success = false;
    let mut in_user = false;
    let mut in_loginid = false;
    let mut username: Option<String> = None;
    let mut loginid: Option<String> = None;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let local = e.local_name();
                let name = String::from_utf8_lossy(local.as_ref());
                match name.as_ref() {
                    "authenticationSuccess" => in_success = true,
                    "user" if in_success => in_user = true,
                    "loginid" if in_success => in_loginid = true,
                    "authenticationFailure" => {
                        return Err(WebError::Unauthorized(
                            "CAS authentication failed".to_string(),
                        ));
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(e)) => {
                let text = e.unescape().map_err(|_| {
                    WebError::CustomApiError(anyhow::anyhow!("Failed to decode CAS response text"))
                })?;
                if in_user {
                    username = Some(text.into_owned());
                } else if in_loginid {
                    loginid = Some(text.into_owned());
                }
            }
            Ok(Event::End(e)) => {
                let local = e.local_name();
                let name = String::from_utf8_lossy(local.as_ref());
                match name.as_ref() {
                    "authenticationSuccess" => in_success = false,
                    "user" => in_user = false,
                    "loginid" => in_loginid = false,
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }

    loginid
        .or(username)
        .ok_or_else(|| WebError::Unauthorized("No username in CAS response".to_string()))
}

fn success_html() -> String {
    r#"<!DOCTYPE html>
<html>
<head>
  <title>Login Success</title>
  <style>
    body, div, p { margin: 0; padding: 0; box-sizing: border-box; }
    body { min-height: 100vh; background: #f7f7f7; display: flex; align-items: center; justify-content: center; }
    .board { padding: 40px; border-radius: 8px; background: #fff; box-shadow: 0 0 60px rgb(84 89 104 / 5%); text-align: center; }
    .icon-success { display: block; width: 18px; height: 38px; margin: 0 auto 16px; border-bottom: 3px solid #07c160; border-right: 3px solid #07c160; transform: rotate(45deg); }
    h2 { margin: 0 0 8px; font-size: 20px; color: #333; }
    p { margin: 0; font-size: 14px; color: #666; }
  </style>
</head>
<body>
  <div class="board">
    <span class="icon-success"></span>
    <h2>Login Success</h2>
    <p>You can close this window now.</p>
  </div>
</body>
</html>"#.to_string()
}
