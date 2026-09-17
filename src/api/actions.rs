//! Versioned action, approval, execution, and receipt endpoints.
//! Used by: api router.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::api::{failpoint_from, Auth};
use crate::domain::{
    ActionContext, ActionIntent, ActionRecord, ActionStatus, ResourceRef, INTENT_VERSION,
};
use crate::error::{Error, Result};
use crate::identity::AuthContext;
use crate::receipt::verify_receipt;
use crate::server::AppState;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposeRequest {
    pub tenant_id: String,
    pub actor: ActorIn,
    pub provider: String,
    pub operation: String,
    pub resource: ResourceIn,
    pub arguments: Value,
    #[serde(default)]
    pub context: Option<ContextIn>,
    #[serde(default)]
    pub ttl_seconds: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorIn {
    pub subject: String,
    pub agent_id: String,
    #[serde(default)]
    pub delegated_by: Option<String>,
    pub issuer: String,
}

#[derive(Deserialize)]
pub struct ResourceIn {
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ContextIn {
    #[serde(default)]
    pub support_ticket_id: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HashBody {
    pub intent_hash: String,
}

#[derive(Default, Deserialize)]
pub struct ExecuteBody {
    #[serde(default)]
    pub arguments: Option<Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionView {
    pub id: Uuid,
    pub status: ActionStatus,
    pub tenant_id: String,
    pub intent_hash: String,
    pub provider: String,
    pub operation: String,
    pub resource: ResourceView,
    pub arguments: Value,
    pub context: ActionContext,
    pub actor: crate::domain::ActorIdentity,
    pub expires_at: String,
    pub policy: Option<crate::domain::PolicyDecision>,
    pub approval: Option<crate::domain::Approval>,
    pub provider_resource_id: Option<String>,
    pub reconciliation_required: bool,
}

#[derive(Serialize)]
pub struct ResourceView {
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
}

impl ActionView {
    fn from_record(record: &ActionRecord) -> Self {
        Self {
            id: record.intent.action_id,
            status: record.status,
            tenant_id: record.intent.tenant_id.clone(),
            intent_hash: record.intent_hash.clone(),
            provider: record.intent.provider.clone(),
            operation: record.intent.operation.clone(),
            resource: ResourceView {
                resource_type: record.intent.resource.resource_type.clone(),
                id: record.intent.resource.resource_id.clone(),
            },
            arguments: record.intent.arguments.clone(),
            context: record.intent.context.clone(),
            actor: record.intent.actor.clone(),
            expires_at: record.intent.expires_at.to_rfc3339(),
            policy: record.policy.clone(),
            approval: record.approval.clone(),
            provider_resource_id: record
                .provider_result
                .as_ref()
                .and_then(|result| result.provider_resource_id.clone()),
            reconciliation_required: record.reconciliation_required,
        }
    }
}

pub async fn propose(
    State(state): State<AppState>,
    Auth(auth): Auth,
    Json(req): Json<ProposeRequest>,
) -> Result<Json<ActionView>> {
    validate_identity(&auth, &req)?;
    let ttl = req.ttl_seconds.unwrap_or(300).clamp(0, 86_400);
    let now = Utc::now();
    let action_id = Uuid::new_v4();
    let intent = ActionIntent {
        version: INTENT_VERSION.to_owned(),
        action_id,
        tenant_id: req.tenant_id,
        actor: auth.actor.clone(),
        provider: req.provider,
        operation: req.operation,
        resource: ResourceRef {
            resource_type: req.resource.resource_type,
            resource_id: req.resource.id,
        },
        arguments: req.arguments,
        context: ActionContext {
            support_ticket_id: req
                .context
                .as_ref()
                .and_then(|c| c.support_ticket_id.clone()),
            reason: req.context.as_ref().and_then(|c| c.reason.clone()),
        },
        idempotency_key: action_id.to_string(),
        created_at: now,
        expires_at: now + Duration::seconds(ttl),
    };
    let record = state.engine.propose(&auth, intent).await?;
    Ok(Json(ActionView::from_record(&record)))
}

pub async fn get_action(
    State(state): State<AppState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<ActionView>> {
    let record = state.engine.get(&auth, id).await?;
    Ok(Json(ActionView::from_record(&record)))
}

pub async fn approve(
    State(state): State<AppState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<HashBody>,
) -> Result<Json<ActionView>> {
    let record = state.engine.approve(&auth, id, &body.intent_hash).await?;
    Ok(Json(ActionView::from_record(&record)))
}

pub async fn deny(
    State(state): State<AppState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<HashBody>,
) -> Result<Json<ActionView>> {
    let record = state.engine.deny(&auth, id, &body.intent_hash).await?;
    Ok(Json(ActionView::from_record(&record)))
}

pub async fn execute(
    State(state): State<AppState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ActionView>> {
    let arguments = if body.is_empty() {
        None
    } else {
        serde_json::from_slice::<ExecuteBody>(&body)
            .map_err(|_| Error::InvalidRequest("invalid json"))?
            .arguments
    };
    let record = state
        .engine
        .execute(&auth, id, arguments, failpoint_from(&headers))
        .await?;
    Ok(Json(ActionView::from_record(&record)))
}

pub async fn reconcile(
    State(state): State<AppState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<ActionView>> {
    let record = state.engine.reconcile(&auth, id).await?;
    Ok(Json(ActionView::from_record(&record)))
}

pub async fn receipt(
    State(state): State<AppState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<crate::domain::SignedReceipt>> {
    let record = state.engine.get(&auth, id).await?;
    let receipt = record.receipt.ok_or(Error::NotFound)?;
    verify_receipt(&receipt, &state.keys)?;
    Ok(Json(receipt))
}

pub async fn keys(State(state): State<AppState>) -> Json<Value> {
    Json(serde_json::json!({
        "keys": [state.keys.public_jwk()]
    }))
}

pub async fn approval_page(
    State(state): State<AppState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
) -> Result<Response> {
    let record = state.engine.get(&auth, id).await?;
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>mint.run approval</title></head><body>\
         <h1>mint.run approval</h1>\
         <dl>\
         <dt>tenant</dt><dd>{}</dd>\
         <dt>actor</dt><dd>{} / {}</dd>\
         <dt>ticket</dt><dd>{}</dd>\
         <dt>charge</dt><dd>{}</dd>\
         <dt>amount</dt><dd>{}</dd>\
         <dt>currency</dt><dd>{}</dd>\
         <dt>reason</dt><dd>{}</dd>\
         <dt>expiration</dt><dd>{}</dd>\
         <dt>intent hash</dt><dd><code>{}</code></dd>\
         </dl>\
         <p>Separate-principal approval: POST /v1/actions/{}/approve with body \
         <code>{{\"intentHash\":\"{}\"}}</code>. Approver subject must differ from the originating actor. \
         The hash is bound to this exact action.</p>\
         </body></html>",
        esc(&record.intent.tenant_id),
        esc(&record.intent.actor.subject),
        esc(&record.intent.actor.agent_id),
        esc(record.intent.context.support_ticket_id.as_deref().unwrap_or("")),
        esc(&record.intent.resource.resource_id),
        esc(&record.intent.arguments.get("amount").map(ToString::to_string).unwrap_or_default()),
        esc(record.intent.arguments.get("currency").and_then(Value::as_str).unwrap_or("")),
        esc(record.intent.arguments.get("reason").and_then(Value::as_str).unwrap_or("")),
        esc(&record.intent.expires_at.to_rfc3339()),
        esc(&record.intent_hash),
        record.intent.action_id,
        esc(&record.intent_hash),
    );
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        Html(html),
    )
        .into_response())
}

fn validate_identity(auth: &AuthContext, req: &ProposeRequest) -> Result<()> {
    if req.tenant_id != auth.tenant_id {
        return Err(Error::Forbidden);
    }
    if req.actor.subject != auth.actor.subject
        || req.actor.agent_id != auth.actor.agent_id
        || req.actor.issuer != auth.actor.issuer
        || req.actor.delegated_by != auth.actor.delegated_by
    {
        return Err(Error::IdentityFailed);
    }
    Ok(())
}

fn esc(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
