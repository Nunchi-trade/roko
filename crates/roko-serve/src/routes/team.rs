//! Team management routes.
//!
//! - `GET    /api/team/me`              — get current user from JWT
//! - `GET    /api/team/members`         — list team members
//! - `POST   /api/team/invite`          — invite a member by email
//! - `PUT    /api/team/members/:did`    — update a member's role
//! - `DELETE /api/team/members/:did`    — remove a member
//!
//! Members are stored in `.roko/team/members.json`.  Invitations are stored
//! in `.roko/team/invitations.json`.  Authorization is entirely local — no
//! Privy API calls are needed.  First user becomes owner automatically.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post, put};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::state::AppState;

// ── Types ───────────────────────────────────────────────────────────

/// A team member record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    /// Unique identity (from JWT `sub` claim, e.g. Privy DID).
    pub id: String,
    /// Email address (may be empty if auth doesn't provide it).
    #[serde(default)]
    pub email: String,
    /// Role: "owner", "admin", "member", "viewer".
    pub role: String,
    /// ISO 8601 timestamp.
    pub joined_at: String,
}

/// A pending invitation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invitation {
    /// Email of the invitee.
    pub email: String,
    /// Role to assign on acceptance.
    pub role: String,
    /// Who invited.
    pub invited_by: String,
    /// ISO 8601 timestamp.
    pub created_at: String,
}

/// Payload for inviting a member.
#[derive(Debug, Deserialize)]
pub struct InviteRequest {
    /// Email to invite.
    pub email: String,
    /// Role to assign (defaults to "member").
    #[serde(default = "default_role")]
    pub role: String,
}

/// Payload for updating a member's role.
#[derive(Debug, Deserialize)]
pub struct UpdateRoleRequest {
    pub role: String,
}

fn default_role() -> String {
    "member".into()
}

// ── Routes ──────────────────────────────────────────────────────────

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/team/me", get(get_me))
        .route("/team/members", get(list_members))
        .route("/team/invite", post(invite_member))
        .route(
            "/team/members/{did}",
            put(update_member).delete(remove_member),
        )
}

// ── Handlers ────────────────────────────────────────────────────────

/// `GET /api/team/me` — return current user from request identity.
///
/// If the user doesn't exist in the members list, auto-creates them.  The
/// first user becomes the owner.
async fn get_me(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let caller = extract_caller_id(&headers);
    let caller_email = extract_caller_email(&headers);

    let members_path = members_path(&state.workdir);
    let mut members = load_members(&members_path);

    // Auto-create on first visit.
    if let Some(existing) = members.iter().find(|m| m.id == caller) {
        return Ok(Json(json!(existing)));
    }

    // First user becomes owner.
    let role: String = if members.is_empty() {
        "owner".into()
    } else {
        // Check if this email was invited.
        let invitations = load_invitations(&state.workdir);
        invitations
            .iter()
            .find(|i| i.email == caller_email)
            .map(|i| i.role.clone())
            .unwrap_or_else(|| "viewer".into())
    };

    let member = TeamMember {
        id: caller.clone(),
        email: caller_email,
        role,
        joined_at: Utc::now().to_rfc3339(),
    };

    members.push(member.clone());
    save_members(&members_path, &members)?;

    // Clean up the invitation if one existed.
    cleanup_invitation(&state.workdir, &member.email);

    Ok(Json(json!(member)))
}

/// `GET /api/team/members` — list all team members.
async fn list_members(State(state): State<Arc<AppState>>) -> Json<Value> {
    let members = load_members(&members_path(&state.workdir));
    let invitations = load_invitations(&state.workdir);
    Json(json!({
        "members": members,
        "invitations": invitations,
    }))
}

/// `POST /api/team/invite` — invite a member by email.
async fn invite_member(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<InviteRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if req.email.trim().is_empty() {
        return Err(ApiError::bad_request("email must not be empty"));
    }

    let valid_roles = ["admin", "member", "viewer"];
    if !valid_roles.contains(&req.role.as_str()) {
        return Err(ApiError::bad_request(format!(
            "role must be one of: {}",
            valid_roles.join(", ")
        )));
    }

    // Check caller has permission (owner or admin).
    let caller = extract_caller_id(&headers);
    let members = load_members(&members_path(&state.workdir));
    require_role(&members, &caller, &["owner", "admin"])?;

    // Check not already a member.
    if members.iter().any(|m| m.email == req.email) {
        return Err(ApiError::conflict("user is already a team member"));
    }

    // Check not already invited.
    let mut invitations = load_invitations(&state.workdir);
    if invitations.iter().any(|i| i.email == req.email) {
        return Err(ApiError::conflict("invitation already pending"));
    }

    let invitation = Invitation {
        email: req.email.clone(),
        role: req.role,
        invited_by: caller,
        created_at: Utc::now().to_rfc3339(),
    };

    invitations.push(invitation.clone());
    save_invitations(&state.workdir, &invitations)?;

    Ok((StatusCode::CREATED, Json(json!(invitation))))
}

/// `PUT /api/team/members/:did` — update a member's role.
async fn update_member(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(did): AxumPath<String>,
    Json(req): Json<UpdateRoleRequest>,
) -> Result<Json<Value>, ApiError> {
    let valid_roles = ["admin", "member", "viewer"];
    if !valid_roles.contains(&req.role.as_str()) {
        return Err(ApiError::bad_request(format!(
            "role must be one of: {}",
            valid_roles.join(", ")
        )));
    }

    let caller = extract_caller_id(&headers);
    let members_path = members_path(&state.workdir);
    let mut members = load_members(&members_path);

    require_role(&members, &caller, &["owner", "admin"])?;

    // Cannot change owner's role.
    let target = members
        .iter_mut()
        .find(|m| m.id == did)
        .ok_or_else(|| ApiError::not_found("member not found"))?;

    if target.role == "owner" {
        return Err(ApiError::bad_request("cannot change owner role"));
    }

    target.role = req.role;
    let updated = target.clone();
    save_members(&members_path, &members)?;

    Ok(Json(json!(updated)))
}

/// `DELETE /api/team/members/:did` — remove a member.
async fn remove_member(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(did): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    let caller = extract_caller_id(&headers);
    let members_path = members_path(&state.workdir);
    let mut members = load_members(&members_path);

    require_role(&members, &caller, &["owner", "admin"])?;

    // Cannot remove yourself.
    if caller == did {
        return Err(ApiError::bad_request("cannot remove yourself"));
    }

    // Cannot remove the owner.
    if members.iter().any(|m| m.id == did && m.role == "owner") {
        return Err(ApiError::bad_request("cannot remove the owner"));
    }

    let before = members.len();
    members.retain(|m| m.id != did);
    if members.len() == before {
        return Err(ApiError::not_found("member not found"));
    }

    save_members(&members_path, &members)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Helpers ─────────────────────────────────────────────────────────

fn team_dir(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("team")
}

fn members_path(workdir: &Path) -> PathBuf {
    team_dir(workdir).join("members.json")
}

fn invitations_path(workdir: &Path) -> PathBuf {
    team_dir(workdir).join("invitations.json")
}

fn load_members(path: &Path) -> Vec<TeamMember> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_members(path: &Path, members: &[TeamMember]) -> Result<(), ApiError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ApiError::internal(format!("create team dir: {e}")))?;
    }
    let data = serde_json::to_string_pretty(members)
        .map_err(|e| ApiError::internal(format!("serialize members: {e}")))?;
    std::fs::write(path, data)
        .map_err(|e| ApiError::internal(format!("write members.json: {e}")))?;
    Ok(())
}

fn load_invitations(workdir: &Path) -> Vec<Invitation> {
    let path = invitations_path(workdir);
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_invitations(workdir: &Path, invitations: &[Invitation]) -> Result<(), ApiError> {
    let path = invitations_path(workdir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ApiError::internal(format!("create team dir: {e}")))?;
    }
    let data = serde_json::to_string_pretty(invitations)
        .map_err(|e| ApiError::internal(format!("serialize invitations: {e}")))?;
    std::fs::write(path, data)
        .map_err(|e| ApiError::internal(format!("write invitations.json: {e}")))?;
    Ok(())
}

fn cleanup_invitation(workdir: &Path, email: &str) {
    let mut invitations = load_invitations(workdir);
    invitations.retain(|i| i.email != email);
    if let Err(err) = save_invitations(workdir, &invitations) {
        tracing::warn!(email = %email, error = %err, "failed to clean up accepted invitation");
    }
}

/// Extract caller identity from headers.
///
/// Checks `X-User-Id` (set by auth middleware when JWT is verified) first,
/// then falls back to a default for unauthenticated local dev.
fn extract_caller_id(headers: &HeaderMap) -> String {
    headers
        .get("x-user-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("local-user")
        .to_string()
}

/// Extract email from headers when available.
fn extract_caller_email(headers: &HeaderMap) -> String {
    headers
        .get("x-user-email")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

/// Verify the caller has one of the required roles.
fn require_role(members: &[TeamMember], caller_id: &str, allowed: &[&str]) -> Result<(), ApiError> {
    let member = members.iter().find(|m| m.id == caller_id);
    match member {
        Some(m) if allowed.contains(&m.role.as_str()) => Ok(()),
        Some(_) => Err(ApiError::forbidden("insufficient permissions")),
        None => Err(ApiError::forbidden("not a team member")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn first_member_becomes_owner() {
        let dir = tempdir().unwrap();
        let path = members_path(dir.path());
        let members = load_members(&path);
        assert!(members.is_empty());

        let member = TeamMember {
            id: "user-1".into(),
            email: "test@example.com".into(),
            role: "owner".into(),
            joined_at: Utc::now().to_rfc3339(),
        };
        save_members(&path, &[member.clone()]).unwrap();

        let loaded = load_members(&path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].role, "owner");
    }

    #[test]
    fn invitation_roundtrip() {
        let dir = tempdir().unwrap();
        let inv = Invitation {
            email: "new@example.com".into(),
            role: "member".into(),
            invited_by: "owner-1".into(),
            created_at: Utc::now().to_rfc3339(),
        };
        save_invitations(dir.path(), &[inv]).unwrap();
        let loaded = load_invitations(dir.path());
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].email, "new@example.com");
    }

    #[test]
    fn cleanup_invitation_removes_by_email() {
        let dir = tempdir().unwrap();
        let invitations = vec![
            Invitation {
                email: "keep@example.com".into(),
                role: "member".into(),
                invited_by: "owner".into(),
                created_at: Utc::now().to_rfc3339(),
            },
            Invitation {
                email: "remove@example.com".into(),
                role: "admin".into(),
                invited_by: "owner".into(),
                created_at: Utc::now().to_rfc3339(),
            },
        ];
        save_invitations(dir.path(), &invitations).unwrap();
        cleanup_invitation(dir.path(), "remove@example.com");
        let loaded = load_invitations(dir.path());
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].email, "keep@example.com");
    }

    #[test]
    fn require_role_rejects_wrong_role() {
        let members = vec![TeamMember {
            id: "user-1".into(),
            email: "test@example.com".into(),
            role: "viewer".into(),
            joined_at: Utc::now().to_rfc3339(),
        }];
        let result = require_role(&members, "user-1", &["owner", "admin"]);
        assert!(result.is_err());
    }

    #[test]
    fn require_role_accepts_correct_role() {
        let members = vec![TeamMember {
            id: "user-1".into(),
            email: "test@example.com".into(),
            role: "admin".into(),
            joined_at: Utc::now().to_rfc3339(),
        }];
        let result = require_role(&members, "user-1", &["owner", "admin"]);
        assert!(result.is_ok());
    }
}
