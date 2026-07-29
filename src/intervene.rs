//! Loopback-only read-only serving for one recorded intervention run.
//! Used by: agentmint-intervene binary and endpoint tests.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rand::RngCore;
use serde::Serialize;

use crate::aerf::intervention::{CorrectionFork, CorrectionPreview, Episode, Evidence, Run};
use crate::fork_from_here::{
    default_executor_for_run_path, CorrectionExecutor, ForkFromHereError, OfflineFixtureExecutor,
};

#[derive(Clone)]
struct InterveneState {
    run: Arc<Run>,
    correction_executor: Arc<dyn CorrectionExecutor + Send + Sync>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceRecord {
    pub episode_id: String,
    pub evidence: Evidence,
}

pub fn build_router(run: Run, token: &str) -> Router {
    let state = build_state(run, Arc::new(OfflineFixtureExecutor));
    let protected = protected_router().with_state(state);

    Router::new()
        .route(&format!("/{token}"), get(get_app_shell))
        .route(&format!("/{token}/"), get(get_app_shell))
        .nest(&format!("/{token}"), protected)
}

pub fn build_router_with_executor(
    run: Run,
    token: &str,
    correction_executor: Arc<dyn CorrectionExecutor + Send + Sync>,
) -> Router {
    let state = build_state(run, correction_executor);
    let protected = protected_router().with_state(state);

    Router::new()
        .route(&format!("/{token}"), get(get_app_shell))
        .route(&format!("/{token}/"), get(get_app_shell))
        .nest(&format!("/{token}"), protected)
}

pub async fn run_with_listener(
    listener: tokio::net::TcpListener,
    run: Run,
    token: &str,
) -> std::io::Result<()> {
    let router = build_router(run, token);
    tracing::info!(
        "agentmint-intervene listening on {:?}",
        listener.local_addr()
    );
    axum::serve(listener, router).await
}

pub async fn run_with_listener_for_path(
    listener: tokio::net::TcpListener,
    run: Run,
    token: &str,
    run_path: &std::path::Path,
    repo_root: &std::path::Path,
) -> std::io::Result<()> {
    let executor =
        default_executor_for_run_path(run_path, repo_root).map_err(std::io::Error::other)?;
    let router = build_router_with_executor(run, token, executor);
    tracing::info!(
        "agentmint-intervene listening on {:?}",
        listener.local_addr()
    );
    axum::serve(listener, router).await
}

pub fn generate_url_token() -> String {
    let mut bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn build_state(
    run: Run,
    correction_executor: Arc<dyn CorrectionExecutor + Send + Sync>,
) -> InterveneState {
    InterveneState {
        run: Arc::new(run),
        correction_executor,
    }
}

fn protected_router() -> Router<InterveneState> {
    Router::new()
        .route("/run", get(get_run))
        .route("/episodes/{episode_id}", get(get_episode))
        .route("/evidence/{evidence_id}", get(get_evidence))
        .route(
            "/corrections/preview/{episode_id}",
            post(preview_correction),
        )
        .route(
            "/corrections/confirm/{episode_id}",
            post(confirm_correction),
        )
}

async fn get_app_shell() -> Response {
    let mut response = Html(app_shell()).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
}

async fn get_run(State(state): State<InterveneState>) -> Json<Run> {
    Json((*state.run).clone())
}

async fn get_episode(
    State(state): State<InterveneState>,
    Path(episode_id): Path<String>,
) -> Result<Json<Episode>, StatusCode> {
    state
        .run
        .episodes
        .iter()
        .find(|episode| episode.episode_id == episode_id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn get_evidence(
    State(state): State<InterveneState>,
    Path(requested_evidence_id): Path<String>,
) -> Result<Json<EvidenceRecord>, StatusCode> {
    state
        .run
        .episodes
        .iter()
        .find_map(|episode| {
            episode
                .evidence
                .iter()
                .find(|evidence| evidence_id(evidence) == requested_evidence_id)
                .cloned()
                .map(|evidence| EvidenceRecord {
                    episode_id: episode.episode_id.clone(),
                    evidence,
                })
        })
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn preview_correction(
    State(state): State<InterveneState>,
    Path(episode_id): Path<String>,
) -> Result<Json<CorrectionPreview>, StatusCode> {
    state
        .correction_executor
        .preview(&state.run, &episode_id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn confirm_correction(
    State(state): State<InterveneState>,
    Path(episode_id): Path<String>,
) -> Result<Json<CorrectionFork>, (StatusCode, Json<serde_json::Value>)> {
    state
        .correction_executor
        .confirm(&state.run, &episode_id)
        .map(Json)
        .map_err(correction_error_response)
}

fn correction_error_response(error: ForkFromHereError) -> (StatusCode, Json<serde_json::Value>) {
    match error {
        ForkFromHereError::UnsupportedEpisode(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": error.to_string() })),
        ),
        ForkFromHereError::Checkpoint(crate::checkpoint::CheckpointError::UnsafeState {
            ref reasons,
            ..
        }) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": error.to_string(),
                "reasons": reasons,
            })),
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        ),
    }
}

fn evidence_id(evidence: &Evidence) -> &str {
    match evidence {
        Evidence::ObservedFact { evidence_id, .. }
        | Evidence::AgentStatement { evidence_id, .. }
        | Evidence::DerivedInference { evidence_id, .. }
        | Evidence::HumanContext { evidence_id, .. } => evidence_id,
    }
}

fn app_shell() -> &'static str {
    r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>AgentMint Intervene</title>
  <style>
    :root {
      color-scheme: light;
      --paper: #f6f1e7;
      --panel: #fffaf1;
      --ink: #1f1d1a;
      --muted: #6d655a;
      --line: #d3c7b4;
      --accent: #c7582b;
      --fact: #1f7a8c;
      --agent: #8f3f71;
      --inference: #d17700;
      --human: #2d6a4f;
      --selected: #1f1d1a;
      --shadow: 0 18px 48px rgba(73, 53, 24, 0.10);
      --radius: 20px;
      --mono: "SFMono-Regular", "SF Mono", ui-monospace, Menlo, monospace;
      --sans: "Iowan Old Style", "Palatino Linotype", "Book Antiqua", Georgia, serif;
    }

    * {
      box-sizing: border-box;
    }

    html, body {
      margin: 0;
      min-height: 100%;
      background:
        radial-gradient(circle at top left, rgba(199, 88, 43, 0.08), transparent 32%),
        linear-gradient(180deg, #fbf7ef 0%, var(--paper) 100%);
      color: var(--ink);
      font-family: var(--sans);
    }

    body {
      min-width: 1280px;
      min-height: 720px;
    }

    .app {
      width: 1280px;
      min-height: 720px;
      margin: 0 auto;
      padding: 24px;
      display: grid;
      grid-template-rows: 132px 188px 1fr;
      gap: 18px;
    }

    .panel {
      background: color-mix(in srgb, var(--panel) 94%, white 6%);
      border: 1px solid rgba(112, 92, 63, 0.18);
      border-radius: var(--radius);
      box-shadow: var(--shadow);
      overflow: hidden;
    }

    .header {
      display: grid;
      grid-template-columns: 2.25fr 1fr;
    }

    .header-main {
      padding: 22px 24px;
      border-right: 1px solid rgba(112, 92, 63, 0.14);
    }

    .eyebrow {
      display: inline-flex;
      gap: 8px;
      align-items: center;
      font-size: 12px;
      letter-spacing: 0.16em;
      text-transform: uppercase;
      color: var(--accent);
      font-weight: 700;
    }

    h1 {
      margin: 10px 0 8px;
      font-size: 34px;
      line-height: 1;
      font-weight: 700;
    }

    .summary {
      margin: 0;
      color: var(--muted);
      font-size: 16px;
      line-height: 1.35;
      min-height: 42px;
    }

    .header-meta {
      display: grid;
      grid-template-columns: 1fr 1fr;
      padding: 18px;
      gap: 10px;
      align-content: center;
      background:
        linear-gradient(180deg, rgba(199, 88, 43, 0.04), rgba(199, 88, 43, 0)),
        transparent;
    }

    .stat {
      padding: 12px 14px;
      border: 1px solid rgba(112, 92, 63, 0.14);
      border-radius: 14px;
      background: rgba(255, 255, 255, 0.55);
    }

    .stat-label {
      display: block;
      font-size: 11px;
      text-transform: uppercase;
      letter-spacing: 0.12em;
      color: var(--muted);
      margin-bottom: 6px;
    }

    .stat-value {
      font-size: 20px;
      font-weight: 700;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .strip {
      padding: 18px 18px 14px;
      display: grid;
      grid-template-rows: auto 1fr;
      gap: 14px;
    }

    .strip-top {
      display: flex;
      justify-content: space-between;
      align-items: center;
      gap: 16px;
    }

    .strip-title {
      font-size: 18px;
      font-weight: 700;
    }

    .strip-help {
      font-size: 13px;
      color: var(--muted);
      white-space: nowrap;
    }

    .strip-list {
      display: grid;
      grid-template-columns: repeat(6, minmax(0, 1fr));
      gap: 12px;
      min-height: 124px;
    }

    .episode-tab {
      appearance: none;
      border: 1px solid rgba(112, 92, 63, 0.14);
      background:
        linear-gradient(180deg, rgba(255,255,255,0.95), rgba(246, 241, 231, 0.92));
      border-radius: 16px;
      padding: 14px 14px 12px;
      text-align: left;
      cursor: pointer;
      display: grid;
      grid-template-rows: auto auto 1fr;
      min-height: 124px;
      transition: transform 120ms ease, border-color 120ms ease, background 120ms ease;
    }

    .episode-tab:hover,
    .episode-tab:focus-visible {
      transform: translateY(-1px);
      border-color: rgba(31, 29, 26, 0.28);
      outline: none;
    }

    .episode-tab.is-selected {
      border-color: rgba(31, 29, 26, 0.72);
      background:
        linear-gradient(180deg, rgba(31, 29, 26, 0.98), rgba(63, 55, 49, 0.98));
      color: #fffaf2;
    }

    .episode-index {
      font-size: 11px;
      text-transform: uppercase;
      letter-spacing: 0.14em;
      opacity: 0.76;
      margin-bottom: 8px;
    }

    .episode-name {
      font-size: 20px;
      font-weight: 700;
      line-height: 1.05;
      margin-bottom: 8px;
    }

    .episode-meta {
      font-size: 13px;
      color: inherit;
      opacity: 0.8;
      align-self: end;
    }

    .canvas {
      display: grid;
      grid-template-columns: minmax(0, 2.1fr) minmax(300px, 0.9fr);
      min-height: 0;
    }

    .canvas-main {
      padding: 18px;
      border-right: 1px solid rgba(112, 92, 63, 0.14);
      display: grid;
      grid-template-rows: auto auto 1fr;
      gap: 14px;
      min-height: 0;
    }

    .canvas-side {
      padding: 18px;
      display: grid;
      grid-template-rows: auto auto 1fr;
      gap: 14px;
      min-height: 0;
      background:
        linear-gradient(180deg, rgba(199, 88, 43, 0.03), rgba(199, 88, 43, 0.00));
    }

    .section-title {
      font-size: 21px;
      font-weight: 700;
      margin: 0;
    }

    .section-copy {
      margin: 0;
      font-size: 14px;
      color: var(--muted);
      line-height: 1.4;
      min-height: 40px;
    }

    .evidence-list,
    .detail-stack,
    .legend,
    .event-list {
      display: grid;
      gap: 12px;
      align-content: start;
    }

    .evidence-list {
      overflow: auto;
      padding-right: 4px;
    }

    .evidence-card,
    .legend-card,
    .event-card {
      border: 1px solid rgba(112, 92, 63, 0.14);
      border-radius: 16px;
      background: rgba(255,255,255,0.70);
    }

    .evidence-card {
      padding: 16px 16px 14px;
      display: grid;
      gap: 10px;
      min-height: 124px;
    }

    .evidence-card.kind-observed_fact { border-left: 8px solid var(--fact); }
    .evidence-card.kind-agent_statement { border-left: 8px solid var(--agent); }
    .evidence-card.kind-derived_inference { border-left: 8px solid var(--inference); }
    .evidence-card.kind-human_context { border-left: 8px solid var(--human); }

    .tag {
      display: inline-flex;
      align-items: center;
      width: fit-content;
      padding: 5px 9px;
      border-radius: 999px;
      font-size: 11px;
      letter-spacing: 0.11em;
      font-weight: 700;
      text-transform: uppercase;
      color: white;
    }

    .tag-fact { background: var(--fact); }
    .tag-agent { background: var(--agent); }
    .tag-inference { background: var(--inference); }
    .tag-human { background: var(--human); }

    .evidence-summary {
      margin: 0;
      font-size: 19px;
      line-height: 1.3;
      font-weight: 700;
    }

    .evidence-meta {
      margin: 0;
      font-size: 13px;
      color: var(--muted);
      font-family: var(--mono);
      word-break: break-word;
    }

    .legend-card,
    .event-card {
      padding: 12px 14px;
    }

    .legend-card {
      display: grid;
      gap: 8px;
    }

    .legend-copy {
      color: var(--muted);
      font-size: 13px;
      line-height: 1.35;
      margin: 0;
    }

    .event-list {
      overflow: auto;
      padding-right: 4px;
    }

    .event-card {
      display: grid;
      gap: 6px;
    }

    .event-id {
      font-family: var(--mono);
      font-size: 13px;
      font-weight: 700;
    }

    .event-copy {
      font-size: 13px;
      color: var(--muted);
      line-height: 1.35;
    }

    .detail-stack {
      overflow: auto;
      padding-right: 4px;
    }

    .detail-card {
      padding: 14px;
      border: 1px solid rgba(112, 92, 63, 0.14);
      border-radius: 16px;
      background: rgba(255,255,255,0.74);
    }

    .detail-card pre {
      margin: 8px 0 0;
      white-space: pre-wrap;
      word-break: break-word;
      font-size: 13px;
      line-height: 1.45;
      font-family: var(--mono);
      color: var(--muted);
    }

    .empty {
      min-height: 160px;
      border: 1px dashed rgba(112, 92, 63, 0.24);
      border-radius: 16px;
      display: grid;
      place-items: center;
      text-align: center;
      color: var(--muted);
      padding: 24px;
      background: rgba(255,255,255,0.48);
    }

    kbd {
      font-family: var(--mono);
      font-size: 11px;
      border-radius: 6px;
      border: 1px solid rgba(112, 92, 63, 0.18);
      padding: 2px 6px;
      background: rgba(255,255,255,0.75);
      color: var(--ink);
    }

    .action-row {
      display: flex;
      gap: 10px;
      flex-wrap: wrap;
      align-items: center;
    }

    .action-button {
      appearance: none;
      border: 1px solid rgba(31, 29, 26, 0.18);
      border-radius: 999px;
      padding: 10px 14px;
      background: rgba(255,255,255,0.9);
      color: var(--ink);
      font: inherit;
      font-size: 14px;
      font-weight: 700;
      cursor: pointer;
    }

    .action-button:hover,
    .action-button:focus-visible {
      border-color: rgba(31, 29, 26, 0.54);
      outline: none;
    }

    .action-button.primary {
      background: linear-gradient(180deg, #2c2a26, #433a34);
      color: #fffaf2;
      border-color: rgba(31, 29, 26, 0.6);
    }

    .action-copy {
      margin: 0;
      color: var(--muted);
      font-size: 13px;
      line-height: 1.35;
    }

    .plain-list {
      margin: 8px 0 0;
      padding-left: 18px;
      color: var(--muted);
      line-height: 1.45;
    }

    .plain-list li + li {
      margin-top: 6px;
    }

    .handoff-grid {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 10px 14px;
      margin-top: 8px;
      margin-bottom: 12px;
    }
  </style>
</head>
<body>
  <div class="app">
    <section class="panel header">
      <div class="header-main">
        <div class="eyebrow">AgentMint Intervene <span id="headerStatus">Recorded Run</span></div>
        <h1 id="runTitle">Loading run…</h1>
        <p class="summary" id="runSummary">Fetching the recorded run and preparing the execution strip.</p>
      </div>
      <div class="header-meta">
        <div class="stat">
          <span class="stat-label">Provider</span>
          <span class="stat-value" id="providerValue">—</span>
        </div>
        <div class="stat">
          <span class="stat-label">Model</span>
          <span class="stat-value" id="modelValue">—</span>
        </div>
        <div class="stat">
          <span class="stat-label">Episodes</span>
          <span class="stat-value" id="episodeCountValue">—</span>
        </div>
        <div class="stat">
          <span class="stat-label">Evidence</span>
          <span class="stat-value" id="evidenceCountValue">—</span>
        </div>
      </div>
    </section>

    <section class="panel strip">
      <div class="strip-top">
        <div class="strip-title">Execution Strip</div>
        <div class="strip-help">Pointer to browse. <kbd>←</kbd>/<kbd>→</kbd>, <kbd>h</kbd>/<kbd>l</kbd>, <kbd>Enter</kbd>, <kbd>Esc</kbd>.</div>
      </div>
      <div class="strip-list" id="episodeStrip" role="tablist" aria-label="Semantic episodes"></div>
    </section>

    <section class="panel canvas">
      <div class="canvas-main">
        <h2 class="section-title" id="canvasTitle">Episode canvas</h2>
        <p class="section-copy" id="canvasCopy">Select an episode to inspect its evidence treatments and retained raw-event IDs.</p>
        <div class="evidence-list" id="evidenceCanvas"></div>
      </div>
      <aside class="canvas-side">
        <h2 class="section-title">Details</h2>
        <p class="section-copy" id="detailCopy">The side rail stays spatially stable while the selected episode changes.</p>
        <div class="action-row" id="actionRow"></div>
        <div class="detail-stack" id="detailStack"></div>
      </aside>
    </section>
  </div>

  <script>
    const state = {
      run: null,
      selectedIndex: 0,
      selectedEpisode: null,
      episodeButtons: [],
      lastFork: null
    };

    const nodes = {
      headerStatus: document.getElementById('headerStatus'),
      runTitle: document.getElementById('runTitle'),
      runSummary: document.getElementById('runSummary'),
      providerValue: document.getElementById('providerValue'),
      modelValue: document.getElementById('modelValue'),
      episodeCountValue: document.getElementById('episodeCountValue'),
      evidenceCountValue: document.getElementById('evidenceCountValue'),
      strip: document.getElementById('episodeStrip'),
      canvasTitle: document.getElementById('canvasTitle'),
      canvasCopy: document.getElementById('canvasCopy'),
      evidenceCanvas: document.getElementById('evidenceCanvas'),
      detailCopy: document.getElementById('detailCopy'),
      actionRow: document.getElementById('actionRow'),
      detailStack: document.getElementById('detailStack')
    };

    const schemaEpisodeId = 'episode-edit-schema';

    function evidenceKindLabel(kind) {
      if (kind === 'observed_fact') return 'FACT';
      if (kind === 'agent_statement') return 'AGENT STATEMENT';
      if (kind === 'derived_inference') return 'INFERENCE';
      return 'HUMAN CONTEXT';
    }

    function evidenceKindClass(kind) {
      if (kind === 'observed_fact') return 'tag-fact';
      if (kind === 'agent_statement') return 'tag-agent';
      if (kind === 'derived_inference') return 'tag-inference';
      return 'tag-human';
    }

    function episodeTitle(episode, index) {
      return `${index + 1}. ${episode.title}`;
    }

    async function loadRun() {
      const response = await fetch('./run');
      if (!response.ok) throw new Error(`Run request failed with ${response.status}`);
      const run = await response.json();
      state.run = run;
      state.selectedIndex = 0;
      state.lastFork = null;
      renderRunHeader();
      renderEpisodeStrip();
      await selectEpisode(0, false);
    }

    function renderRunHeader() {
      const run = state.run;
      const evidenceCount = run.episodes.reduce((total, episode) => total + episode.evidence.length, 0);
      let shape = 'records the original six-step intervention flow.';
      if (run.run_id.endsWith('-fixture-fork')) {
        shape = 'includes a deterministic fixture-backed corrected branch from the schema episode.';
      } else if (run.run_id.endsWith('-codex-fork')) {
        shape = 'includes a live Codex-backed corrected branch with a terminal handoff.';
      }
      nodes.headerStatus.textContent = run.verification.verdict.replaceAll('_', ' ');
      nodes.runTitle.textContent = run.run_id;
      nodes.runSummary.textContent = `${run.session_id} ${shape}`;
      nodes.providerValue.textContent = run.provider;
      nodes.modelValue.textContent = run.model;
      nodes.episodeCountValue.textContent = `${run.episodes.length} locked`;
      nodes.evidenceCountValue.textContent = `${evidenceCount} cards`;
    }

    function renderEpisodeStrip() {
      nodes.strip.innerHTML = '';
      state.episodeButtons = [];
      state.run.episodes.forEach((episode, index) => {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'episode-tab';
        button.setAttribute('role', 'tab');
        button.setAttribute('aria-selected', 'false');
        button.dataset.index = String(index);
        button.innerHTML = `
          <div class="episode-index">Episode ${index + 1}</div>
          <div class="episode-name">${episode.title}</div>
          <div class="episode-meta">${episode.raw_event_ids.length} raw events • ${episode.evidence.length} evidence items</div>
        `;
        button.addEventListener('pointerdown', () => selectEpisode(index, true));
        button.addEventListener('click', () => selectEpisode(index, true));
        nodes.strip.appendChild(button);
        state.episodeButtons.push(button);
      });
      updateStripSelection();
    }

    function updateStripSelection() {
      state.episodeButtons.forEach((button, index) => {
        const selected = index === state.selectedIndex;
        button.classList.toggle('is-selected', selected);
        button.setAttribute('aria-selected', selected ? 'true' : 'false');
        button.tabIndex = selected ? 0 : -1;
      });
    }

    async function selectEpisode(index, focus) {
      if (!state.run) return;
      const clamped = Math.max(0, Math.min(index, state.run.episodes.length - 1));
      state.selectedIndex = clamped;
      updateStripSelection();
      if (focus && state.episodeButtons[clamped]) {
        state.episodeButtons[clamped].focus();
      }
      const episode = state.run.episodes[clamped];
      const response = await fetch(`./episodes/${episode.episode_id}`);
      if (!response.ok) throw new Error(`Episode request failed with ${response.status}`);
      state.selectedEpisode = await response.json();
      renderEpisodeCanvas();
      renderActionRow();
      if (state.lastFork && state.lastFork.appended_episode_id === state.selectedEpisode.episode_id) {
        renderConfirmedForkDetail(state.lastFork);
        return;
      }
      renderDetailRail();
    }

    function renderEpisodeCanvas() {
      const episode = state.selectedEpisode;
      nodes.canvasTitle.textContent = episodeTitle(episode, state.selectedIndex);
      nodes.canvasCopy.textContent = `This canvas stays fixed while you move across the execution strip. Raw events retained: ${episode.raw_event_ids.join(', ')}.`;
      nodes.evidenceCanvas.innerHTML = '';

      if (!episode.evidence.length) {
        nodes.evidenceCanvas.innerHTML = '<div class="empty">No evidence items were recorded for this episode.</div>';
        return;
      }

      episode.evidence.forEach((item) => {
        const card = document.createElement('article');
        card.className = `evidence-card kind-${item.kind}`;
        const tagClass = evidenceKindClass(item.kind);
        const meta = evidenceMeta(item);
        card.innerHTML = `
          <span class="tag ${tagClass}">${evidenceKindLabel(item.kind)}</span>
          <p class="evidence-summary">${item.summary}</p>
          <p class="evidence-meta">${meta}</p>
        `;
        nodes.evidenceCanvas.appendChild(card);
      });
    }

    function evidenceMeta(item) {
      if (item.kind === 'observed_fact' || item.kind === 'agent_statement') {
        return `Source event: ${item.source_event_id}`;
      }
      if (item.kind === 'derived_inference') {
        return `Derived from: ${item.derived_from.join(', ')}`;
      }
      return `Provided by: ${item.provided_by}`;
    }

    function renderDetailRail() {
      const episode = state.selectedEpisode;
      nodes.detailCopy.textContent = `${episode.raw_event_ids.length} raw-event IDs remain visible even when low-signal inspections collapse into one semantic step.`;
      nodes.detailStack.innerHTML = '';

      const legend = document.createElement('section');
      legend.className = 'legend';
      legend.innerHTML = `
        <div class="legend-card">
          <span class="tag tag-fact">FACT</span>
          <p class="legend-copy">Directly observed evidence tied to a source event.</p>
        </div>
        <div class="legend-card">
          <span class="tag tag-agent">AGENT STATEMENT</span>
          <p class="legend-copy">Claims or verbalized reasoning attributed to the agent.</p>
        </div>
        <div class="legend-card">
          <span class="tag tag-inference">INFERENCE</span>
          <p class="legend-copy">Derived interpretation assembled from prior evidence.</p>
        </div>
        <div class="legend-card">
          <span class="tag tag-human">HUMAN CONTEXT</span>
          <p class="legend-copy">Operator-provided context that remains distinct from system facts.</p>
        </div>
      `;

      const events = document.createElement('section');
      events.className = 'event-list';
      episode.raw_event_ids.forEach((eventId) => {
        const event = state.run.raw_events.find((candidate) => candidate.event_id === eventId);
        const card = document.createElement('article');
        card.className = 'event-card';
        const title = event ? (event.summary || event.endpoint) : 'Missing event';
        const path = event && event.file_path ? event.file_path : 'no file path';
        card.innerHTML = `
          <div class="event-id">${eventId}</div>
          <div class="event-copy">${title}</div>
          <div class="event-copy">${path}</div>
        `;
        events.appendChild(card);
      });
      const limitationCard = document.createElement('section');
      limitationCard.className = 'detail-card';
      limitationCard.innerHTML = `<strong>Recorded-run limitations</strong><ul class="plain-list">
        <li>Episode evidence and raw events come from the recorded run payload, not from a live workspace replay.</li>
        <li>Malformed or truncated raw events remain grouped deterministically from the parser output instead of being reinterpreted in the browser.</li>
      </ul>`;

      nodes.detailStack.append(legend, events, limitationCard);
    }

    function renderActionRow() {
      nodes.actionRow.innerHTML = '';
      const episode = state.selectedEpisode;
      if (!episode || episode.episode_id !== schemaEpisodeId) {
        return;
      }

      if (state.run.run_id.endsWith('-fixture-fork')) {
        const note = document.createElement('p');
        note.className = 'action-copy';
        note.textContent = 'This run already includes the fixture-backed corrected attempt branch.';
        nodes.actionRow.appendChild(note);
        return;
      }

      const previewButton = document.createElement('button');
      previewButton.type = 'button';
      previewButton.className = 'action-button';
      previewButton.textContent = 'Preview correction packet';
      previewButton.addEventListener('click', () => {
        previewCorrection().catch(renderActionError);
      });

      const help = document.createElement('p');
      help.className = 'action-copy';
      help.textContent = 'Preview the correction packet first. Fixture runs stay deterministic; non-fixture runs can hand off into an isolated Codex worktree.';

      nodes.actionRow.append(previewButton, help);
    }

    async function previewCorrection() {
      const episode = state.selectedEpisode;
      const response = await fetch(`./corrections/preview/${episode.episode_id}`, {
        method: 'POST'
      });
      if (!response.ok) {
        throw new Error(`Correction preview failed with ${response.status}`);
      }
      const preview = await response.json();
      renderCorrectionPreview(preview);
    }

    function renderCorrectionPreview(preview) {
      nodes.detailStack.innerHTML = '';

      const packetCard = document.createElement('section');
      packetCard.className = 'detail-card';
      packetCard.innerHTML = `<strong>Correction packet preview</strong><pre>${JSON.stringify(preview.packet, null, 2)}</pre>`;

      const comparisonCard = document.createElement('section');
      comparisonCard.className = 'detail-card';
      comparisonCard.innerHTML = `<strong>Original versus corrected comparison</strong><pre>${JSON.stringify({
        original_attempt_id: preview.comparison.original_attempt_id,
        corrected_attempt: preview.comparison.corrected_attempt,
        patch: preview.comparison.patch,
        verification: preview.comparison.verification,
        token_usage: preview.comparison.token_usage
      }, null, 2)}</pre>`;

      const confirmCard = document.createElement('section');
      confirmCard.className = 'detail-card';
      const confirmButton = document.createElement('button');
      confirmButton.type = 'button';
      confirmButton.className = 'action-button primary';
      confirmButton.textContent = 'Fork from here';
      confirmButton.addEventListener('click', () => {
        confirmCorrection(preview.branch_from_episode_id).catch(renderActionError);
      });
      const copy = document.createElement('p');
      copy.className = 'action-copy';
      copy.textContent = preview.packet.packet_id === 'packet-fixture-correction-from-schema'
        ? 'This stays in offline deterministic demo mode. No real worktree, Codex thread, or terminal handoff will be created.'
        : 'This will create an isolated worktree, start a Codex correction attempt, run the recorded verification command, and return a terminal handoff.';

      const limits = document.createElement('ul');
      limits.className = 'plain-list';
      if (preview.packet.packet_id === 'packet-fixture-correction-from-schema') {
        limits.innerHTML = `
          <li>The corrected attempt is appended only inside the browser session.</li>
          <li>No checkpoint, branch, or verification command is executed in the repository.</li>
        `;
      } else {
        limits.innerHTML = `
          <li>The live path still relies on the installed Codex App Server protocol shape.</li>
          <li>The recorded verification command is inferred from the fixture test metadata.</li>
        `;
      }
      confirmCard.append(confirmButton, copy, limits);

      nodes.detailStack.append(packetCard, comparisonCard, confirmCard);
    }

    async function confirmCorrection(episodeId) {
      const response = await fetch(`./corrections/confirm/${episodeId}`, {
        method: 'POST'
      });
      if (!response.ok) {
        throw new Error(`Correction confirmation failed with ${response.status}`);
      }
      const fork = await response.json();
      state.lastFork = {
        appended_episode_id: fork.appended_episode_id,
        packet: fork.packet,
        comparison: fork.comparison,
        execution: fork.execution
      };
      state.run = fork.forked_run;
      const nextIndex = state.run.episodes.findIndex((episode) => episode.episode_id === fork.appended_episode_id);
      state.selectedIndex = nextIndex >= 0 ? nextIndex : state.run.episodes.length - 1;
      renderRunHeader();
      renderEpisodeStrip();
      await selectEpisode(state.selectedIndex, true);
      renderConfirmedForkDetail(fork);
    }

    function renderConfirmedForkDetail(fork) {
      nodes.detailStack.innerHTML = '';
      if (!fork.execution) {
        const detail = document.createElement('section');
        detail.className = 'detail-card';
        detail.innerHTML = `<strong>Offline deterministic branch</strong><ul class="plain-list">
          <li>Appended episode: ${fork.appended_episode_id}</li>
          <li>Packet: ${fork.packet.packet_id}</li>
          <li>No real directory, branch, checkpoint, Codex thread, or verification command was created.</li>
          <li>This fixture branch exists only in the current browser session.</li>
        </ul>`;
        nodes.detailStack.appendChild(detail);
        return;
      }

      const execution = fork.execution;
      const handoff = execution.handoff;

      const handoffCard = document.createElement('section');
      handoffCard.className = 'detail-card';
      handoffCard.innerHTML = `
        <strong>Terminal handoff</strong>
        <div class="handoff-grid">
          <div><span class="stat-label">Directory</span><div class="event-copy">${handoff.directory}</div></div>
          <div><span class="stat-label">Branch State</span><div class="event-copy">${handoff.branch_state}</div></div>
          <div><span class="stat-label">Checkpoint</span><div class="event-copy">${handoff.checkpoint_id}</div></div>
          <div><span class="stat-label">Attempt</span><div class="event-copy">${handoff.attempt_id}</div></div>
        </div>
      `;
      const handoffPre = document.createElement('pre');
      handoffPre.textContent = handoff.copy_text;
      const copyButton = document.createElement('button');
      copyButton.type = 'button';
      copyButton.className = 'action-button primary';
      copyButton.textContent = 'Copy terminal handoff';
      copyButton.addEventListener('click', async () => {
        await copyText(handoff.copy_text);
        copyButton.textContent = 'Copied terminal handoff';
        window.setTimeout(() => {
          copyButton.textContent = 'Copy terminal handoff';
        }, 1200);
      });
      handoffCard.append(handoffPre, copyButton);

      const comparisonCard = document.createElement('section');
      comparisonCard.className = 'detail-card';
      comparisonCard.innerHTML = `
        <strong>Patch and token comparison</strong>
        <pre>${JSON.stringify({
          patch: fork.comparison.patch,
          token_usage: fork.comparison.token_usage,
          verification: fork.comparison.verification
        }, null, 2)}</pre>
      `;

      const limitationsCard = document.createElement('section');
      limitationsCard.className = 'detail-card';
      limitationsCard.innerHTML = `<strong>Remaining demo limitations</strong>${renderPlainList(execution.demo_limitations)}`;

      nodes.detailStack.append(handoffCard, comparisonCard, limitationsCard);
    }

    function renderActionError(error) {
      nodes.detailStack.innerHTML = `<div class="empty">${error.message}</div>`;
    }

    async function copyText(value) {
      if (navigator.clipboard && navigator.clipboard.writeText) {
        await navigator.clipboard.writeText(value);
        return;
      }
      const area = document.createElement('textarea');
      area.value = value;
      area.setAttribute('readonly', 'true');
      area.style.position = 'absolute';
      area.style.left = '-9999px';
      document.body.appendChild(area);
      area.select();
      document.execCommand('copy');
      document.body.removeChild(area);
    }

    function renderPlainList(items) {
      return `<ul class="plain-list">${items.map((item) => `<li>${item}</li>`).join('')}</ul>`;
    }

    function moveSelection(delta) {
      if (!state.run) return;
      const next = (state.selectedIndex + delta + state.run.episodes.length) % state.run.episodes.length;
      selectEpisode(next, true);
    }

    function activateSelection() {
      if (!state.run) return;
      selectEpisode(state.selectedIndex, true);
    }

    function resetSelection() {
      if (!state.run) return;
      selectEpisode(0, true);
    }

    document.addEventListener('keydown', (event) => {
      if (!state.run) return;
      if (event.key === 'ArrowLeft' || event.key === 'h') {
        event.preventDefault();
        moveSelection(-1);
      } else if (event.key === 'ArrowRight' || event.key === 'l') {
        event.preventDefault();
        moveSelection(1);
      } else if (event.key === 'Enter') {
        event.preventDefault();
        activateSelection();
      } else if (event.key === 'Escape') {
        event.preventDefault();
        resetSelection();
      }
    });

    loadRun().catch((error) => {
      nodes.runTitle.textContent = 'Run load failed';
      nodes.runSummary.textContent = error.message;
      nodes.evidenceCanvas.innerHTML = '<div class="empty">The UI could not load the recorded run.</div>';
    });
  </script>
</body>
</html>"#
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;

    use reqwest::StatusCode;

    use super::{build_router_with_executor, run_with_listener};
    use crate::aerf::intervention::{
        CorrectionExecution, CorrectionExecutionMode, CorrectionFork, CorrectionPreview, Run,
        TerminalHandoff, TokenUsage, RECORDED_DEMO_SCHEMA_EPISODE_ID,
    };
    use crate::fork_from_here::{CorrectionExecutor, ForkFromHereError, OfflineFixtureExecutor};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    async fn spawn_server() -> Result<(String, reqwest::Client), Box<dyn std::error::Error>> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/interventions/retry-after-v1-v2.json");
        let run = Run::from_slice(&fs::read(path)?)?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        tokio::spawn(run_with_listener(listener, run, "test-token"));
        let client = reqwest::Client::builder().no_proxy().build()?;
        Ok((format!("http://{addr}/test-token"), client))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_route_returns_recorded_run() -> TestResult {
        let (base, client) = spawn_server().await?;

        let response = client.get(format!("{base}/run")).send().await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body: Run = response.json().await?;
        assert_eq!(body.run_id, "run-retry-after-v1-v2");
        assert_eq!(body.episodes.len(), 6);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn app_shell_route_returns_html_ui() -> TestResult {
        let (base, client) = spawn_server().await?;

        let response = client.get(format!("{base}/")).send().await?;

        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok());
        assert_eq!(content_type, Some("text/html; charset=utf-8"));
        let body = response.text().await?;
        assert!(body.contains("Execution Strip"));
        assert!(body.contains("AGENT STATEMENT"));
        assert!(body.contains("Copy terminal handoff"));
        assert!(body.contains("Remaining demo limitations"));
        assert!(body.contains("ArrowLeft"));
        assert!(body.contains("ArrowRight"));
        assert!(body.contains("Escape"));
        assert!(body.contains("copyText"));
        assert!(body.contains("./run"));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn episode_route_returns_selected_episode() -> TestResult {
        let (base, client) = spawn_server().await?;

        let response = client
            .get(format!("{base}/episodes/episode-inspect-api"))
            .send()
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(body["title"], serde_json::json!("inspect API"));
        assert_eq!(
            body["raw_event_ids"],
            serde_json::json!(["inspect-api-1", "inspect-api-2", "inspect-api-3"])
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn evidence_route_returns_selected_evidence() -> TestResult {
        let (base, client) = spawn_server().await?;

        let response = client
            .get(format!("{base}/evidence/episode-revise-fixture-observed"))
            .send()
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(
            body["episode_id"],
            serde_json::json!("episode-revise-fixture")
        );
        assert_eq!(
            body["evidence"]["summary"],
            serde_json::json!("Revised the recorded fixture after the failing test.")
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn token_is_required_in_url() -> TestResult {
        let (base, client) = spawn_server().await?;
        let unprotected = base.replacen("/test-token", "", 1);

        let response = client.get(format!("{unprotected}/run")).send().await?;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unknown_resources_return_not_found() -> TestResult {
        let (base, client) = spawn_server().await?;

        let episode = client
            .get(format!("{base}/episodes/does-not-exist"))
            .send()
            .await?;
        let evidence = client
            .get(format!("{base}/evidence/does-not-exist"))
            .send()
            .await?;

        assert_eq!(episode.status(), StatusCode::NOT_FOUND);
        assert_eq!(evidence.status(), StatusCode::NOT_FOUND);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn correction_preview_and_confirm_routes_return_demo_payloads() -> TestResult {
        let (base, client) = spawn_server().await?;

        let preview = client
            .post(format!(
                "{base}/corrections/preview/{}",
                RECORDED_DEMO_SCHEMA_EPISODE_ID
            ))
            .send()
            .await?;
        let confirm = client
            .post(format!(
                "{base}/corrections/confirm/{}",
                RECORDED_DEMO_SCHEMA_EPISODE_ID
            ))
            .send()
            .await?;

        assert_eq!(preview.status(), StatusCode::OK);
        assert_eq!(confirm.status(), StatusCode::OK);

        let preview_json: serde_json::Value = preview.json().await?;
        let confirm_json: serde_json::Value = confirm.json().await?;

        assert_eq!(
            preview_json["packet"]["packet_id"],
            serde_json::json!("packet-fixture-correction-from-schema")
        );
        assert_eq!(
            confirm_json["appended_episode_id"],
            serde_json::json!("episode-fixture-corrected-attempt")
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn live_confirm_route_returns_terminal_handoff_payload() -> TestResult {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/interventions/retry-after-v1-v2.json");
        let run = Run::from_slice(&fs::read(path)?)?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let router = build_router_with_executor(
            run.clone(),
            "test-token",
            Arc::new(MockLiveExecutor {
                preview: CorrectionPreview {
                    branch_from_episode_id: RECORDED_DEMO_SCHEMA_EPISODE_ID.to_owned(),
                    packet: crate::aerf::intervention::generate_live_correction_preview(
                        &run,
                        RECORDED_DEMO_SCHEMA_EPISODE_ID,
                    )
                    .expect("preview")
                    .packet,
                    comparison: crate::aerf::intervention::generate_live_correction_preview(
                        &run,
                        RECORDED_DEMO_SCHEMA_EPISODE_ID,
                    )
                    .expect("preview")
                    .comparison,
                },
                fork: live_fork(run),
            }),
        );
        tokio::spawn(async move {
            axum::serve(listener, router).await.expect("serve router");
        });
        let client = reqwest::Client::builder().no_proxy().build()?;

        let response = client
            .post(format!(
                "http://{addr}/test-token/corrections/confirm/{}",
                RECORDED_DEMO_SCHEMA_EPISODE_ID
            ))
            .send()
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(
            body["execution"]["handoff"]["directory"],
            serde_json::json!("/tmp/agentmint-worktree")
        );
        assert_eq!(
            body["execution"]["handoff"]["branch_state"],
            serde_json::json!("branch agentmint/attempt-codex-corrected-from-schema")
        );
        assert_eq!(
            body["execution"]["handoff"]["verification_command"],
            serde_json::json!(["cargo", "test", "--test", "intervention_parsing"])
        );
        assert!(body["execution"]["handoff"]["copy_text"]
            .as_str()
            .expect("copy text")
            .contains("cd /tmp/agentmint-worktree"));
        Ok(())
    }

    struct MockLiveExecutor {
        preview: CorrectionPreview,
        fork: CorrectionFork,
    }

    impl CorrectionExecutor for MockLiveExecutor {
        fn preview(&self, _run: &Run, episode_id: &str) -> Option<CorrectionPreview> {
            if episode_id == RECORDED_DEMO_SCHEMA_EPISODE_ID {
                return Some(self.preview.clone());
            }
            None
        }

        fn confirm(
            &self,
            _run: &Run,
            episode_id: &str,
        ) -> Result<CorrectionFork, ForkFromHereError> {
            if episode_id == RECORDED_DEMO_SCHEMA_EPISODE_ID {
                return Ok(self.fork.clone());
            }
            Err(ForkFromHereError::UnsupportedEpisode(episode_id.to_owned()))
        }
    }

    fn live_fork(run: Run) -> CorrectionFork {
        let mut fork = OfflineFixtureExecutor
            .confirm(&run, RECORDED_DEMO_SCHEMA_EPISODE_ID)
            .expect("fixture fork");
        fork.execution = Some(CorrectionExecution {
            mode: CorrectionExecutionMode::LiveCodex,
            checkpoint_id: "checkpoint-abc123".to_owned(),
            worktree_path: "/tmp/agentmint-worktree".to_owned(),
            branch_name: "agentmint/attempt-codex-corrected-from-schema".to_owned(),
            detached_head: false,
            codex_version: "0.130.0".to_owned(),
            thread_id: Some("thread-1".to_owned()),
            attempt_id: "attempt-codex-corrected-from-schema".to_owned(),
            verification_command: vec![
                "cargo".to_owned(),
                "test".to_owned(),
                "--test".to_owned(),
                "intervention_parsing".to_owned(),
            ],
            verification_exit_code: 0,
            actual_patch: "diff --git a/src/schema.rs b/src/schema.rs".to_owned(),
            actual_changed_paths: vec!["src/schema.rs".to_owned()],
            captured_token_usage: TokenUsage {
                input_tokens: 40,
                cached_input_tokens: 20,
                output_tokens: 10,
                total_tokens: 50,
            },
            handoff: TerminalHandoff {
                directory: "/tmp/agentmint-worktree".to_owned(),
                branch_state: "branch agentmint/attempt-codex-corrected-from-schema".to_owned(),
                checkpoint_id: "checkpoint-abc123".to_owned(),
                attempt_id: "attempt-codex-corrected-from-schema".to_owned(),
                verification_command: vec![
                    "cargo".to_owned(),
                    "test".to_owned(),
                    "--test".to_owned(),
                    "intervention_parsing".to_owned(),
                ],
                copy_text: "cd /tmp/agentmint-worktree\n# branch state: branch agentmint/attempt-codex-corrected-from-schema\n# checkpoint: checkpoint-abc123\n# attempt: attempt-codex-corrected-from-schema\ncargo test --test intervention_parsing".to_owned(),
            },
            demo_limitations: vec![
                "The browser UI shows the completed handoff after the run returns; it does not live-stream token usage or command output incrementally.".to_owned(),
            ],
            codex_transcript: crate::aerf::adapters::codex_app_server::CodexTranscript {
                codex_version: "0.130.0".to_owned(),
                handshake: crate::aerf::adapters::codex_app_server::CodexInitializeHandshake {
                    codex_version: "0.130.0".to_owned(),
                    initialize_request: serde_json::json!({"method":"initialize"}),
                    initialize_response: serde_json::json!({"result":{"serverInfo":{"version":"0.130.0"}}}),
                    initialized_notification: serde_json::json!({"method":"initialized"}),
                },
                events: Vec::new(),
            },
        });
        fork
    }
}
