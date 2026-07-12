const app = document.querySelector("#app");
const toastRoot = document.querySelector("#toast-root");

const state = {
  data: null,
  view: "dashboard",
  settingsTab: "general",
  templateEditor: null,
  templateOriginalSlug: null,
  templateEditorSection: "basics",
  templateSlugTouched: false,
  templateDirty: false,
  templateFiles: null,
  templateFilesError: "",
  savingTemplate: false,
  selectedTemplate: null,
  variables: {},
  preview: null,
  previewError: "",
  previewTimer: null,
  search: "",
  searchResults: null,
  searchTimer: null,
  creating: false,
  reconciling: false,
  moving: false,
  selected: new Set(),
  bulkBase: "",
  gitInit: false,
  reveal: true,
  detail: null,
  detailBusy: false,
  apply: null,
  modal: null,
  register: {
    path: "",
    template: "",
    variables: {},
    rename: false,
    apply: false,
    useToday: false,
    created: "",
    busy: false,
  },
  appearance: loadAppearance(),
};

function loadAppearance() {
  const defaults = { theme: "system", accent: "lime", density: "comfortable" };
  try {
    return { ...defaults, ...JSON.parse(localStorage.getItem("fastf-appearance") || "{}") };
  } catch {
    return defaults;
  }
}

function applyAppearance() {
  const prefersDark = window.matchMedia?.("(prefers-color-scheme: dark)").matches;
  const resolvedTheme = state.appearance.theme === "system"
    ? (prefersDark ? "dark" : "light")
    : state.appearance.theme;
  document.documentElement.dataset.theme = resolvedTheme;
  document.documentElement.dataset.accent = state.appearance.accent;
  document.documentElement.dataset.density = state.appearance.density;
}

applyAppearance();

const initialParams = new URLSearchParams(window.location.search);
const initialView = initialParams.get("view");
const initialTemplate = initialParams.get("template");
const initialSettingsTab = initialParams.get("settings");
const initialEditTemplate = initialParams.get("edit-template");
const initialEditorSection = initialParams.get("editor-section");
if (["dashboard", "create", "template-editor", "templates", "projects", "register", "settings"].includes(initialView)) {
  state.view = initialView;
}
if (["general", "data", "appearance"].includes(initialSettingsTab)) {
  state.settingsTab = initialSettingsTab;
}
if (["basics", "variables", "structure", "files", "automation"].includes(initialEditorSection)) {
  state.templateEditorSection = initialEditorSection;
}

const icons = {
  home: '<path d="m3 10 9-7 9 7v9a2 2 0 0 1-2 2h-4v-7H9v7H5a2 2 0 0 1-2-2v-9Z"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  layers: '<path d="m12 2 9 5-9 5-9-5 9-5Z"/><path d="m3 12 9 5 9-5M3 17l9 5 9-5"/>',
  folder: '<path d="M3 6a2 2 0 0 1 2-2h5l2 2h7a2 2 0 0 1 2 2v9a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3V6Z"/>',
  settings: '<path d="M12 15.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7Z"/><path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.83 2.83-.06-.06a1.7 1.7 0 0 0-1.88-.34 1.7 1.7 0 0 0-1.03 1.55V21h-4v-.08A1.7 1.7 0 0 0 8.95 19.4a1.7 1.7 0 0 0-1.88.34l-.06.06-2.83-2.83.06-.06A1.7 1.7 0 0 0 4.58 15 1.7 1.7 0 0 0 3.03 14H3v-4h.08A1.7 1.7 0 0 0 4.6 8.95a1.7 1.7 0 0 0-.34-1.88l-.06-.06 2.83-2.83.06.06A1.7 1.7 0 0 0 9 4.58 1.7 1.7 0 0 0 10 3.03V3h4v.08A1.7 1.7 0 0 0 15.05 4.6a1.7 1.7 0 0 0 1.88-.34l.06-.06 2.83 2.83-.06.06A1.7 1.7 0 0 0 19.42 9 1.7 1.7 0 0 0 20.97 10H21v4h-.08A1.7 1.7 0 0 0 19.4 15Z"/>',
  search: '<circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/>',
  bolt: '<path d="m13 2-9 12h7l-1 8 9-13h-7l1-7Z"/>',
  arrow: '<path d="M5 12h14M13 6l6 6-6 6"/>',
  grid: '<rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/>',
  clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
  file: '<path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8Z"/><path d="M14 2v6h6"/>',
  image: '<rect x="3" y="3" width="18" height="18" rx="3"/><circle cx="8.5" cy="8.5" r="1.5"/><path d="m21 15-5-5L5 21"/>',
  video: '<rect x="3" y="5" width="14" height="14" rx="2"/><path d="m17 10 4-3v10l-4-3"/>',
  code: '<path d="m8 9-4 3 4 3M16 9l4 3-4 3M14 5l-4 14"/>',
  check: '<path d="m5 12 4 4L19 6"/>',
  external: '<path d="M15 3h6v6M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>',
  refresh: '<path d="M20 7h-6V1"/><path d="M20 7a9 9 0 1 0 1 8"/>',
  sliders: '<path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6"/>',
  tag: '<path d="M20.6 13.6 11 4H4v7l9.6 9.6a2 2 0 0 0 2.8 0l4.2-4.2a2 2 0 0 0 0-2.8Z"/><circle cx="7.5" cy="7.5" r=".8" fill="currentColor"/>',
  more: '<circle cx="5" cy="12" r="1" fill="currentColor"/><circle cx="12" cy="12" r="1" fill="currentColor"/><circle cx="19" cy="12" r="1" fill="currentColor"/>',
  chevron: '<path d="m9 18 6-6-6-6"/>',
  spark: '<path d="m12 3 1.4 4.1L17.5 8.5l-4.1 1.4L12 14l-1.4-4.1-4.1-1.4 4.1-1.4L12 3ZM19 15l.8 2.2L22 18l-2.2.8L19 21l-.8-2.2L16 18l2.2-.8L19 15Z"/>',
  database: '<ellipse cx="12" cy="5" rx="8" ry="3"/><path d="M4 5v7c0 1.7 3.6 3 8 3s8-1.3 8-3V5"/><path d="M4 12v7c0 1.7 3.6 3 8 3s8-1.3 8-3v-7"/>',
  palette: '<path d="M12 3a9 9 0 0 0 0 18h1.5a2 2 0 0 0 0-4H12a2 2 0 0 1 0-4h3a6 6 0 0 0 0-12h-3Z"/><circle cx="7.5" cy="10" r="1" fill="currentColor"/><circle cx="9" cy="6.5" r="1" fill="currentColor"/><circle cx="14" cy="6" r="1" fill="currentColor"/>',
  info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v5M12 8h.01"/>',
  folderPlus: '<path d="M3 6a2 2 0 0 1 2-2h5l2 2h7a2 2 0 0 1 2 2v9a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3V6Z"/><path d="M12 11v6M9 14h6"/>',
  download: '<path d="M12 3v12M7 11l5 5 5-5"/><path d="M5 21h14"/>',
  upload: '<path d="M12 21V9M7 13l5-5 5 5"/><path d="M5 3h14"/>',
  copy: '<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>',
  close: '<path d="M6 6l12 12M18 6 6 18"/>',
  trash: '<path d="M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2M6 7l1 13a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1l1-13"/>',
  note: '<path d="M5 4a1 1 0 0 1 1-1h9l4 4v13a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1Z"/><path d="M14 3v5h5M8 13h8M8 17h5"/>',
};

function icon(name, className = "") {
  return `<svg class="${className}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${icons[name] || icons.folder}</svg>`;
}

function esc(value = "") {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

async function api(route, options = {}) {
  const response = await fetch(route, {
    headers: { "Content-Type": "application/json" },
    ...options,
  });
  const payload = await response.json();
  if (!response.ok || payload.ok === false) {
    throw new Error(payload.error || "Fast Folder could not complete the request.");
  }
  return payload;
}

function toast(message, error = false) {
  const node = document.createElement("div");
  node.className = `toast${error ? " error" : ""}`;
  node.innerHTML = `${icon(error ? "info" : "check")}<span>${esc(message)}</span>`;
  toastRoot.append(node);
  setTimeout(() => node.remove(), 3600);
}

function templateVisual(template) {
  const slug = template.slug.toLowerCase();
  if (slug.includes("photo")) return { icon: "image", color: "purple" };
  if (slug.includes("video") || slug.includes("music")) return { icon: "video", color: "blue" };
  if (slug.includes("rust") || slug.includes("web") || slug.includes("code")) return { icon: "code", color: "orange" };
  return { icon: "folder", color: "" };
}

function shortPath(path) {
  if (!path) return "Current folder";
  return path.replace(/^\/home\/[^/]+/, "~");
}

function formatDate(value) {
  if (!value) return "Unknown";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value.slice(0, 10);
  return new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", year: "numeric" }).format(date);
}

function formatBytes(bytes = 0) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function shell(content) {
  const data = state.data;
  return `
    <div class="shell">
      <aside class="sidebar">
        <div class="brand">
          <div class="brand-mark"></div>
          <div class="brand-copy"><strong>Fast Folder</strong><span>Project systems</span></div>
        </div>
        <div class="nav-label">Workspace</div>
        <nav class="nav">
          ${navButton("dashboard", "home", "Overview")}
          ${navButton("create", "plus", "New project")}
          ${navButton("register", "folderPlus", "Add existing")}
          ${navButton("templates", "layers", "Templates")}
          ${navButton("projects", "folder", "Projects")}
        </nav>
        <div class="nav-label" style="margin-top:24px">System</div>
        <nav class="nav">${navButton("settings", "settings", "Settings")}</nav>
        <div class="sidebar-spacer"></div>
        <div class="storage-card">
          <div class="eyebrow">Project base</div>
          <div class="storage-path" title="${esc(data.config.base_dir)}">${esc(shortPath(data.config.base_dir))}</div>
          <div class="storage-meta"><span>Next project</span><strong>${esc(data.next_id)}</strong></div>
        </div>
      </aside>
      <main class="main">
        <header class="topbar">
          <div class="search">
            ${icon("search")}
            <input id="global-search" value="${esc(state.search)}" placeholder="Search projects and templates" autocomplete="off">
            <span class="search-kbd">⌘ K</span>
          </div>
          <div class="top-spacer"></div>
          <button class="icon-button" data-action="refresh" title="Refresh">${icon("refresh")}</button>
          <div class="id-pill">
            <div class="id-dot">${icon("bolt")}</div>
            <div class="id-copy"><span>Ready</span><strong>${esc(data.next_id)}</strong></div>
          </div>
        </header>
        <div class="content">${content}</div>
      </main>
    </div>`;
}

// Banner shown when any project has an interrupted copy or move (durable marker
// on disk). Retry runs the same reconcile the `fastf reconcile` CLI does.
function provisioningBanner() {
  const items = state.data?.provisioning || [];
  if (!items.length) return "";
  const noun = items.length === 1 ? "project needs" : "projects need";
  const detail = items
    .map((it) => `${baseLabel(it.path.replace(/\/[^/]*$/, "")) || it.path} (${it.kind})`)
    .slice(0, 3)
    .join(", ");
  return `
    <div class="recover-banner">
      <div class="recover-copy">
        ${icon("bolt")}
        <div>
          <strong>${items.length} ${noun} recovery</strong>
          <span>An asset copy or move was interrupted — ${esc(detail)}. Your data is safe; finish it now.</span>
        </div>
      </div>
      <button class="button button-dark" data-reconcile ${state.reconciling ? "disabled" : ""}>
        ${icon("refresh")} ${state.reconciling ? "Reconciling…" : "Reconcile"}
      </button>
    </div>`;
}

function navButton(view, iconName, label) {
  const active = state.view === view || (view === "templates" && state.view === "template-editor");
  return `<button class="nav-button ${active ? "active" : ""}" data-view="${view}" title="${label}">${icon(iconName)}<span>${label}</span></button>`;
}

function render() {
  if (!state.data) return;
  let content;
  if (state.view === "create") content = createPage();
  else if (state.view === "template-editor") content = templateEditorPage();
  else if (state.view === "templates") content = templatesPage();
  else if (state.view === "projects") content = projectsPage();
  else if (state.view === "register") content = registerPage();
  else if (state.view === "settings") content = settingsPage();
  else content = dashboardPage();
  app.innerHTML = shell(provisioningBanner() + content) + drawerMarkup() + applyModalMarkup() + genericModalMarkup();
  bindCommon();
  bindView();
  bindDrawer();
  bindApplyModal();
  bindGenericModal();
}

function dashboardPage() {
  const data = state.data;
  const existingProjects = data.projects.filter((project) => project.exists);
  const latest = existingProjects[0];
  return `
    <section class="page">
      <div class="page-header">
        <div>
          <h1 class="page-title">Good ${dayPart()}.</h1>
          <p class="page-subtitle">Your project workspace is ready. Start from a template or continue where you left off.</p>
        </div>
      </div>
      <div class="hero">
        <div class="hero-copy">
          <div class="eyebrow">Fast, consistent, organized</div>
          <h2>Build the whole project in one move.</h2>
          <p>Choose a system, add the details, and Fast Folder creates every directory, file, ID, and piece of metadata exactly where it belongs.</p>
          <button class="button button-primary" data-start-create>${icon("plus")} Create a project</button>
        </div>
        <div class="hero-art">
          <div class="floating-tree">
            <div class="floating-tree-head"><i></i><span>${esc(latest?.name || "Your next project")}</span></div>
            <div class="floating-line"></div><div class="floating-line"></div><div class="floating-line"></div><div class="floating-line"></div><div class="floating-line"></div>
          </div>
        </div>
      </div>
      <div class="dashboard-grid">
        ${statCard("folder", "", data.projects.length, "Projects tracked")}
        ${statCard("layers", "green", data.templates.length, "Templates ready")}
        ${statCard("bolt", "blue", data.next_id, "Next project ID")}
        ${statCard("clock", "purple", latest ? formatDate(latest.created_at) : "No projects", "Last created")}
      </div>
      <section class="section">
        <div class="section-head"><h2 class="section-title">Recent projects</h2><button class="button button-quiet" data-view="projects">View all ${icon("arrow")}</button></div>
        <div class="panel project-list">${projectRows(data.projects.slice(0, 6))}</div>
      </section>
      ${quickStartSection()}
    </section>`;
}

function quickStartSection() {
  const templates = state.data.templates.filter((template) => template.slug !== "(registered)").slice(0, 4);
  if (!templates.length) return "";
  return `
    <section class="section">
      <div class="section-head"><h2 class="section-title">Quick start</h2><button class="button button-quiet" data-view="templates">All templates ${icon("arrow")}</button></div>
      <div class="quick-start">
        ${templates.map(quickStartChip).join("")}
      </div>
    </section>`;
}

function quickStartChip(template) {
  const visual = templateVisual(template);
  return `
    <button class="quick-chip" data-template="${esc(template.slug)}" title="Create a ${esc(template.name)} project">
      <span class="quick-chip-icon ${visual.color}">${icon(visual.icon)}</span>
      <span class="quick-chip-copy"><strong>${esc(template.name)}</strong><span>${template.variables.length} field${template.variables.length === 1 ? "" : "s"}</span></span>
      <span class="quick-chip-go">${icon("plus")}</span>
    </button>`;
}

function dayPart() {
  const hour = new Date().getHours();
  if (hour < 12) return "morning";
  if (hour < 18) return "afternoon";
  return "evening";
}

function statCard(iconName, color, value, label) {
  return `<div class="stat-card"><div class="stat-icon ${color}">${icon(iconName)}</div><div class="stat-value" title="${esc(value)}">${esc(value)}</div><div class="stat-label">${label}</div></div>`;
}

function countFolders(nodes = []) {
  return nodes.reduce((count, node) => count + 1 + countFolders(node.children || []), 0);
}

function projectRows(projects) {
  if (!projects.length) {
    return `<div class="empty"><div class="empty-icon">${icon("folder")}</div><strong>No projects yet</strong><p>Create your first project and it will appear here automatically.</p></div>`;
  }
  return projects.map((project) => `
    <div class="project-row ${state.selected.has(project.path) ? "selected" : ""}" data-detail-path="${esc(project.path)}" role="button" tabindex="0" title="View project details">
      <label class="project-select" title="Select for bulk move">
        <input type="checkbox" data-select-path="${esc(project.path)}" ${state.selected.has(project.path) ? "checked" : ""}>
      </label>
      <div class="project-name">
        <div class="project-folder ${project.exists ? "" : "missing"}">${icon("folder")}</div>
        <div class="project-name-copy">
          <strong title="${esc(project.name)}">${esc(project.name)}</strong>
          <span title="${esc(project.path)}">${esc(shortPath(project.path))}</span>
          ${(project.tags || []).length ? `<div class="row-tags">${(project.tags || []).slice(0, 3).map((tag) => `<span class="row-tag">${esc(tag)}</span>`).join("")}${project.tags.length > 3 ? `<span class="row-tag more">+${project.tags.length - 3}</span>` : ""}</div>` : ""}
        </div>
      </div>
      <div class="project-cell base-cell" title="${esc(project.base || "")}"><span class="chip">${esc(project.base_label || "—")}</span></div>
      <div class="project-cell">${esc(templateName(project.template))}</div>
      <div class="project-cell">${esc(formatDate(project.created_at))}</div>
      <div><span class="status ${project.exists ? "" : "missing"}">${project.exists ? "Available" : "Missing"}</span></div>
      <button class="row-action" data-open-path="${esc(project.path)}" ${project.exists ? "" : "disabled"} title="Open in file manager">${icon("external")}</button>
    </div>`).join("");
}

function templateName(slug) {
  return state.data.templates.find((template) => template.slug === slug)?.name || slug;
}

// All configured bases (base_dir first, then the extra `bases` list), deduped
// on a trailing-slash-insensitive key. Mirrors Config::effective_bases().
function effectiveBases() {
  const config = state.data.config;
  const seen = new Set();
  const out = [];
  for (const raw of [config.base_dir, ...(config.bases || [])]) {
    const value = (raw || "").trim();
    if (!value) continue;
    const key = value.replace(/\/+$/, "") || "/";
    if (!seen.has(key)) {
      seen.add(key);
      out.push(value);
    }
  }
  return out;
}

function baseLabel(base) {
  const trimmed = (base || "").replace(/\/+$/, "");
  return trimmed.split("/").pop() || base;
}

function createPage() {
  const templates = state.data.templates;
  const selected = templates.find((template) => template.slug === state.selectedTemplate) || templates[0];
  if (selected && state.selectedTemplate !== selected.slug) {
    state.selectedTemplate = selected.slug;
    initializeVariables(selected);
    queueMicrotask(updatePreview);
  }
  return `
    <section class="page">
      <div class="page-header">
        <div><h1 class="page-title">Create a project</h1><p class="page-subtitle">Select a project system, fill in its details, and review the exact output before creating anything.</p></div>
      </div>
      <div class="create-layout">
        <div class="create-pane scroll">
          <div class="pane-title"><strong>Templates</strong><span>${templates.length} available</span></div>
          <div class="template-picker">${templates.map((template) => templateOption(template, selected)).join("")}</div>
        </div>
        <div class="create-pane scroll">
          <div class="pane-title"><strong>Project details</strong><span>Step 2 of 2</span></div>
          <form id="create-form" class="form-body">
            <div class="form-intro"><div class="eyebrow">${esc(selected?.slug)}</div><h2>${esc(selected?.name)}</h2><p>${esc(selected?.description || "Configure the details for this project.")}</p></div>
            ${selected?.variables.length ? selected.variables.map(variableField).join("") : `<div class="empty" style="padding:20px 0 30px"><strong>No details required</strong><p>This template is ready to create as-is.</p></div>`}
            <div class="field">
              <div class="field-row"><label for="base-dir">Create inside</label><span class="field-hint">${effectiveBases().length > 1 ? "Pick a base for this project" : "Configured base"}</span></div>
              <select class="input mono" id="base-dir">
                ${effectiveBases().map((base, index) => `<option value="${esc(base)}" ${index === 0 ? "selected" : ""}>${esc(baseLabel(base))} — ${esc(base)}</option>`).join("")}
              </select>
            </div>
            <div class="options-box">
              ${toggleRow("git-init", "Initialize Git repository", "Run git init after creation", state.gitInit)}
              ${toggleRow("reveal-folder", "Open folder when ready", "Reveal the project in your file manager", state.reveal)}
            </div>
            <div id="create-error">${state.previewError ? `<div class="error-box" style="margin-top:14px">${esc(state.previewError)}</div>` : ""}</div>
            <div class="create-actions">
              <span class="field-hint">Nothing is written until you create.</span>
              <button class="button button-primary" type="submit" ${state.creating ? "disabled" : ""}>${state.creating ? "Creating…" : `${icon("bolt")} Create project`}</button>
            </div>
          </form>
        </div>
        <div class="create-pane preview-pane">${previewPanel()}</div>
      </div>
    </section>`;
}

function templateOption(template, selected) {
  const visual = templateVisual(template);
  return `
    <div class="template-option ${template.slug === selected?.slug ? "active" : ""}" data-select-template="${esc(template.slug)}">
      <div class="template-icon ${visual.color}">${icon(visual.icon)}</div>
      <div class="template-option-copy"><strong>${esc(template.name)}</strong><span>${template.variables.length} fields · ${countFolders(template.structure)} folders</span></div>
    </div>`;
}

function initializeVariables(template) {
  state.variables = Object.fromEntries(template.variables.map((variable) => [variable.slug, variable.default || ""]));
  state.preview = null;
  state.previewError = "";
}

function variableField(variable) {
  const value = state.variables[variable.slug] ?? variable.default ?? "";
  const variableType = variable.type ?? variable.var_type ?? "text";
  const field = variableType === "select"
    ? `<select class="select" data-variable="${esc(variable.slug)}">${variable.options.map((option) => `<option value="${esc(option)}" ${value === option ? "selected" : ""}>${esc(option)}</option>`).join("")}</select>`
    : `<input class="input" data-variable="${esc(variable.slug)}" value="${esc(value)}" placeholder="${esc(variable.label)}" autocomplete="off">`;
  return `<div class="field"><div class="field-row"><label>${esc(variable.label)}${variable.required ? '<span class="required">*</span>' : ""}</label><span class="field-hint">${esc(variable.transform === "none" ? "As entered" : variable.transform.replaceAll("_", " "))}</span></div>${field}</div>`;
}

function toggleRow(id, title, description, checked) {
  return `<div class="toggle-row"><div class="toggle-copy"><strong>${title}</strong><span>${description}</span></div><label class="switch"><input id="${id}" type="checkbox" ${checked ? "checked" : ""}><span></span></label></div>`;
}

function previewPanel() {
  const preview = state.preview;
  if (!preview) {
    return `<div class="preview"><div class="preview-head"><strong>Live preview</strong><span class="preview-badge"><i></i>Waiting for details</span></div><div></div><div class="preview-placeholder">${icon("layers")}<p>Complete the required fields to see the exact folder system Fast Folder will create.</p></div></div>`;
  }
  return `
    <div class="preview">
      <div class="preview-head"><strong>Live preview</strong><span class="preview-badge"><i></i>Ready to create</span></div>
      <div class="preview-path"><div class="eyebrow">Destination</div><strong title="${esc(preview.root_path)}">${esc(shortPath(preview.root_path))}</strong></div>
      <div class="preview-body">
        <div class="tree-root">${icon("folder")}<span>${esc(preview.folder_name)}/</span></div>
        <div class="tree">${treeMarkup(preview.folders)}${preview.files.map((file) => `<div class="tree-node"><div class="tree-label file">${esc(file.path)}</div></div>`).join("")}</div>
      </div>
    </div>`;
}

function treeMarkup(nodes = []) {
  return nodes.map((node) => `<div class="tree-node"><div class="tree-label folder">${esc(node.name)}/</div>${node.children?.length ? `<div class="tree-children">${treeMarkup(node.children)}</div>` : ""}</div>`).join("");
}

function templatesPage() {
  const filtered = filterTemplates();
  return `
    <section class="page">
      <div class="page-header">
        <div><h1 class="page-title">Templates</h1><p class="page-subtitle">Reusable systems that define naming, questions, folders, files, tags, and project automation.</p></div>
        <div class="page-actions">
          <button class="button button-secondary" data-from-folder>${icon("folder")} From folder</button>
          <button class="button button-primary" data-new-template>${icon("plus")} New template</button>
        </div>
      </div>
      <div class="templates-page-grid">
        ${filtered.map((template) => {
          const visual = templateVisual(template);
          return `<article class="template-detail-card">
            <div class="template-card-top">
              <div style="display:flex;gap:11px;align-items:center"><div class="template-icon ${visual.color}">${icon(visual.icon)}</div><div><h3>${esc(template.name)}</h3><div class="slug">${esc(template.slug)}</div></div></div>
              <span class="arrow-chip">${icon("layers")}</span>
            </div>
            <p>${esc(template.description || "A reusable project system ready for your workflow.")}</p>
            <div class="detail-stats">
              <div class="detail-stat"><strong>${template.variables.length}</strong><span>Fields</span></div>
              <div class="detail-stat"><strong>${countFolders(template.structure)}</strong><span>Folders</span></div>
              <div class="detail-stat"><strong>${template.file_count ?? 0}</strong><span>Files</span></div>
            </div>
            <div class="card-actions">
              <button class="button button-secondary" data-edit-template="${esc(template.slug)}">${icon("sliders")} Edit</button>
              <button class="button button-primary" data-template="${esc(template.slug)}">${icon("plus")} Use</button>
            </div>
          </article>`;
        }).join("") || emptySearch("No templates match your search.")}
      </div>
    </section>`;
}

function newTemplateDraft() {
  return {
    name: "Untitled template",
    slug: "untitled-template",
    description: "",
    version: "1",
    naming_pattern: "{date}_{name}_{id}",
    id: { prefix: "ID", digits: 4 },
    variables: [{
      slug: "name",
      label: "Project name",
      type: "text",
      required: true,
      options: [],
      default: "",
      transform: "title_underscore",
    }],
    structure: [],
    verbatim: [],
    exclude: [],
    post_create: null,
    tags: [],
    tag_from: [],
  };
}

function openTemplateEditor(slug = null) {
  const source = slug
    ? state.data.templates.find((template) => template.slug === slug)
    : newTemplateDraft();
  if (!source) {
    toast("That template no longer exists.", true);
    return;
  }
  state.templateEditor = structuredClone(source);
  state.templateOriginalSlug = slug;
  state.templateEditorSection = "basics";
  state.templateSlugTouched = Boolean(slug);
  state.templateDirty = false;
  state.templateFiles = null;
  state.templateFilesError = "";
  state.view = "template-editor";
  render();
  if (slug) loadTemplateFiles(slug);
}

// Fetch a template's real files/ subtree (paths, sizes, and text content) for
// the editor's Files section. Re-renders when done if the editor is open.
async function loadTemplateFiles(slug) {
  try {
    const result = await api(`/api/template-files?slug=${encodeURIComponent(slug)}`);
    state.templateFiles = result.files;
    state.templateFilesError = "";
  } catch (error) {
    state.templateFiles = [];
    state.templateFilesError = error.message;
  }
  if (state.view === "template-editor") render();
}

function templateEditorPage() {
  const template = state.templateEditor;
  if (!template) {
    state.view = "templates";
    return templatesPage();
  }
  const isEditing = Boolean(state.templateOriginalSlug);
  return `
    <section class="page template-editor-page">
      <div class="page-header">
        <div>
          <button class="back-link" data-cancel-template>${icon("arrow")} Back to templates</button>
          <h1 class="page-title">${isEditing ? "Edit template" : "Create a template"}</h1>
          <p class="page-subtitle">Design a complete reusable project system without editing YAML.</p>
        </div>
        <div class="page-actions">
          ${isEditing ? `<button class="button button-danger-quiet" data-delete-template>${icon("more")} Delete</button>` : ""}
          <button class="button button-primary" data-save-template ${state.savingTemplate ? "disabled" : ""}>${icon("check")} ${state.savingTemplate ? "Saving…" : "Save template"}</button>
        </div>
      </div>
      <div class="template-editor-layout">
        <aside class="editor-sidebar panel">
          <div class="editor-summary">
            <div class="template-icon ${templateVisual(template).color}">${icon(templateVisual(template).icon)}</div>
            <strong>${esc(template.name || "Untitled template")}</strong>
            <span>${esc(template.slug || "no-slug")}</span>
          </div>
          <nav class="editor-nav">
            ${editorNavButton("basics", "settings", "Basics")}
            ${editorNavButton("variables", "sliders", "Questions", template.variables.length)}
            ${editorNavButton("structure", "folder", "Folders", countFolders(template.structure))}
            ${editorNavButton("files", "file", "Files", state.templateFiles?.length ?? template.file_count ?? 0)}
            ${editorNavButton("automation", "bolt", "Tags & actions")}
          </nav>
          <div class="editor-token-card">
            <div class="eyebrow">Built-in tokens</div>
            <code>{date}</code><code>{YYYY}</code><code>{MM}</code><code>{DD}</code><code>{id}</code>
          </div>
        </aside>
        <form id="template-editor-form" class="editor-main panel">
          ${templateEditorSection(template)}
        </form>
      </div>
    </section>`;
}

function editorNavButton(section, iconName, label, count = null) {
  return `<button type="button" class="editor-nav-button ${state.templateEditorSection === section ? "active" : ""}" data-editor-section="${section}">${icon(iconName)}<span>${label}</span>${count === null ? "" : `<b>${count}</b>`}</button>`;
}

function templateEditorSection(template) {
  if (state.templateEditorSection === "variables") return variableEditor(template);
  if (state.templateEditorSection === "structure") return structureEditor(template);
  if (state.templateEditorSection === "files") return fileEditor(template);
  if (state.templateEditorSection === "automation") return automationEditor(template);
  return basicTemplateEditor(template);
}

function editorSectionHeader(eyebrow, title, description, action = "") {
  return `<div class="editor-section-head"><div><div class="eyebrow">${eyebrow}</div><h2>${title}</h2><p>${description}</p></div>${action}</div>`;
}

function basicTemplateEditor(template) {
  return `
    ${editorSectionHeader("Template 01", "Basics", "Name the template and define how every generated project folder should be named.")}
    <div class="editor-form-grid">
      ${editorField("Template name", "The name people see in the UI", `<input class="input" id="template-name" value="${esc(template.name)}">`, true)}
      ${editorField("Slug", "Filename and CLI identifier", `<input class="input mono" id="template-slug" value="${esc(template.slug)}">`, true)}
      ${editorField("Description", "A short explanation of this project system", `<textarea class="textarea" id="template-description" rows="3">${esc(template.description)}</textarea>`, false, "full")}
      ${editorField("Naming pattern", "Use built-in tokens and question slugs", `<input class="input mono" id="template-pattern" value="${esc(template.naming_pattern)}">`, true, "full")}
    </div>
    <div class="editor-subsection">
      <h3>Project ID</h3>
      <p>Every Fast Folder project shares one global counter. This controls how that number appears for this template.</p>
      <div class="editor-form-grid">
        ${editorField("Prefix", "Text before the number", `<input class="input mono" id="template-id-prefix" value="${esc(template.id?.prefix || "ID")}">`)}
        ${editorField("Digits", "Zero-padded number width", `<input class="input" type="number" min="1" max="12" id="template-id-digits" value="${esc(template.id?.digits || 4)}">`)}
      </div>
      <div class="pattern-preview">
        <span>Example result</span>
        <strong>${esc(templatePatternExample(template))}</strong>
      </div>
    </div>`;
}

function editorField(label, hint, control, required = false, className = "") {
  return `<div class="editor-field ${className}"><div class="field-row"><label>${label}${required ? '<span class="required">*</span>' : ""}</label><span class="field-hint">${hint}</span></div>${control}</div>`;
}

function templatePatternExample(template) {
  const values = Object.fromEntries((template.variables || []).map((variable) => [variable.slug, variable.default || variable.slug]));
  values.id = `${template.id?.prefix || "ID"}${String(state.data.counter + 1).padStart(template.id?.digits || 4, "0")}`;
  const date = new Date().toISOString().slice(0, 10);
  return (template.naming_pattern || "{date}_{id}")
    .replaceAll("{date}", date)
    .replaceAll("{YYYY}", date.slice(0, 4))
    .replaceAll("{MM}", date.slice(5, 7))
    .replaceAll("{DD}", date.slice(8, 10))
    .replace(/\{([^}]+)\}/g, (_, token) => values[token] || token);
}

function variableEditor(template) {
  return `
    ${editorSectionHeader("Template 02", "Questions", "Define the information Fast Folder asks for when someone creates a project.", `<button type="button" class="button button-primary" data-add-variable>${icon("plus")} Add question</button>`)}
    <div class="editor-stack">
      ${template.variables.length ? template.variables.map(variableEditorCard).join("") : editorEmpty("sliders", "No questions yet", "Add a question when project names, folders, or files need custom information.")}
    </div>`;
}

function variableEditorCard(variable, index) {
  const isSelect = (variable.type ?? variable.var_type) === "select";
  const autoTag = state.templateEditor.tag_from?.includes(variable.slug);
  return `
    <article class="editor-item-card">
      <div class="editor-item-head">
        <div><span>Question ${index + 1}</span><strong>${esc(variable.label || variable.slug || "Untitled question")}</strong></div>
        <button type="button" class="row-action danger" data-remove-variable="${index}" title="Remove question">${icon("more")}</button>
      </div>
      <div class="editor-form-grid">
        ${editorField("Label", "Shown to the user", `<input class="input" data-variable-index="${index}" data-variable-field="label" value="${esc(variable.label)}">`, true)}
        ${editorField("Slug", "Token used as {slug}", `<input class="input mono" data-variable-index="${index}" data-variable-field="slug" value="${esc(variable.slug)}">`, true)}
        ${editorField("Input type", "Text entry or fixed choices", `<select class="select" data-variable-index="${index}" data-variable-field="type"><option value="text" ${isSelect ? "" : "selected"}>Text</option><option value="select" ${isSelect ? "selected" : ""}>Dropdown</option></select>`)}
        ${editorField("Transform", "Formatting applied before generation", `<select class="select" data-variable-index="${index}" data-variable-field="transform">${transformOptions(variable.transform)}</select>`)}
        ${editorField("Default value", "Pre-filled when creating a project", `<input class="input" data-variable-index="${index}" data-variable-field="default" value="${esc(variable.default || "")}">`, false, isSelect ? "" : "full")}
        ${isSelect ? editorField("Dropdown choices", "Separate choices with commas", `<input class="input" data-variable-index="${index}" data-variable-field="options" value="${esc((variable.options || []).join(", "))}">`, true) : ""}
      </div>
      <div class="editor-inline-options">
        ${editorCheck(`variable-required-${index}`, "Required", "A value must be provided", Boolean(variable.required), `data-variable-required="${index}"`)}
        ${editorCheck(`variable-tag-${index}`, "Create a tag", `Store values as ${esc(variable.slug || "slug")}/value`, autoTag, `data-variable-tag="${index}"`)}
      </div>
    </article>`;
}

function transformOptions(selected = "none") {
  return [
    ["none", "As entered"],
    ["title_underscore", "Title underscore"],
    ["upper_underscore", "Upper underscore"],
    ["lower_underscore", "Lower underscore"],
  ].map(([value, label]) => `<option value="${value}" ${selected === value ? "selected" : ""}>${label}</option>`).join("");
}

function editorCheck(id, label, hint, checked, attributes = "") {
  return `<label class="editor-check" for="${id}"><span><strong>${label}</strong><small>${hint}</small></span><span class="switch"><input id="${id}" type="checkbox" ${checked ? "checked" : ""} ${attributes}><span></span></span></label>`;
}

function structureEditor(template) {
  const paths = flattenFolderPaths(template.structure).join("\n");
  return `
    ${editorSectionHeader("Template 03", "Folder structure", "Enter one folder path per line. Use / to describe nesting on every operating system.")}
    <div class="structure-editor-grid">
      <div>
        ${editorField("Folder paths", "Tokens such as {client} are supported", `<textarea class="textarea mono structure-textarea" id="template-folders" rows="18" placeholder="01_Assets/01_Footage&#10;01_Assets/02_Audio&#10;02_Exports">${esc(paths)}</textarea>`, false)}
        <div class="editor-tip">${icon("info")} Existing paths are merged automatically, so shared parent folders only appear once.</div>
      </div>
      <div class="structure-preview-card">
        <div class="eyebrow">Structure preview</div>
        <div id="template-structure-preview" class="light-tree">${template.structure.length ? lightTreeMarkup(template.structure) : `<span class="field-hint">Add folder paths to see the tree.</span>`}</div>
      </div>
    </div>`;
}

function fileEditor(template) {
  const head = editorSectionHeader("Template 04", "Generated files", "Everything in the template's files/ folder is copied into each new project. Text files interpolate {tokens}; binaries are copied byte-for-byte.");
  // File operations act on disk directly, so the template must already exist.
  if (!state.templateOriginalSlug) {
    return `${head}${editorEmpty("file", "Save the template first", "Files live on disk in this template's files/ folder. Save the template, then reopen it here to add or edit files.")}`;
  }
  if (state.templateFiles === null) {
    return `${head}<div class="drawer-loading">${icon("refresh")} Loading files…</div>`;
  }
  const files = state.templateFiles;
  return `
    ${head}
    ${state.templateFilesError ? `<div class="error-box">${esc(state.templateFilesError)}</div>` : ""}
    <div class="file-add-panel">
      <div class="file-add-row">
        <input class="input mono" id="file-new-path" placeholder="docs/NOTES.md" autocomplete="off">
        <button type="button" class="button button-secondary" data-file-new>${icon("plus")} New text file</button>
      </div>
      <div class="file-add-row">
        <input class="input mono" id="file-src" placeholder="/home/you/asset.mp4 (source on disk)" autocomplete="off">
        <input class="input mono" id="file-dest" placeholder="assets/asset.mp4 (optional)" autocomplete="off">
        <button type="button" class="button button-secondary" data-file-add-path>${icon("upload")} Add from path</button>
      </div>
      <div class="editor-tip">${icon("info")} Or drop files straight into the template's files/ folder — they show up here automatically.</div>
    </div>
    <div class="editor-stack">
      ${files.length ? files.map(fileEditorCard).join("") : editorEmpty("file", "No generated files yet", "Add a text file, bundle an asset from a path, or drop files into the files/ folder.")}
    </div>`;
}

function fileEditorCard(file) {
  const meta = file.is_text ? `Text · ${formatBytes(file.size)}` : `Binary · ${formatBytes(file.size)}`;
  const body = file.is_text
    ? editorField("File content", "Saved on blur; {tokens} interpolate at create time", `<textarea class="textarea mono file-content" data-file-path="${esc(file.path)}" rows="9" placeholder="# {name}">${esc(file.content || "")}</textarea>`, false, "full")
    : `<div class="file-binary-note">${icon("info")} Binary asset — copied into every project exactly as-is.</div>`;
  return `
    <article class="editor-item-card">
      <div class="editor-item-head">
        <div><span>${esc(meta)}</span><strong>${esc(file.path)}</strong></div>
        <button type="button" class="row-action danger" data-file-delete="${esc(file.path)}" title="Remove file">${icon("trash")}</button>
      </div>
      ${body}
    </article>`;
}

function automationEditor(template) {
  const override = template.post_create !== null && template.post_create !== undefined;
  const actions = template.post_create || { git_init: false, reveal: false, open_in_editor: false, print_path: false, commands: [] };
  return `
    ${editorSectionHeader("Template 05", "Tags & actions", "Attach searchable tags, tune how bundled files copy, and optionally override the global actions run after project creation.")}
    <div class="editor-subsection flush">
      ${editorField("Default tags", "Comma-separated tags added to every project", `<input class="input" id="template-tags" value="${esc((template.tags || []).join(", "))}" placeholder="creative, client-work">`, false)}
    </div>
    <div class="editor-subsection">
      <h3>Asset copy rules</h3>
      <p>Glob patterns applied to the files/ folder when creating a project. Match on a filename (<code>*.svg</code>) or a path (<code>docs/*.md</code>).</p>
      <div class="editor-form-grid">
        ${editorField("Copy verbatim", "Copy literally even if it looks like text (keeps {braces})", `<input class="input mono" id="template-verbatim" value="${esc((template.verbatim || []).join(", "))}" placeholder="*.svg">`, false)}
        ${editorField("Exclude", "Never copy these into a project", `<input class="input mono" id="template-exclude" value="${esc((template.exclude || []).join(", "))}" placeholder=".DS_Store, *.tmp">`, false)}
      </div>
    </div>
    <div class="editor-subsection">
      ${editorCheck("template-override-actions", "Override global post-create actions", "Use settings specific to this template", override, "data-template-override-actions")}
      <div class="action-grid ${override ? "" : "disabled"}">
        ${editorCheck("template-action-git", "Initialize Git", "Run git init in the project", Boolean(actions.git_init), "data-template-action=\"git_init\"")}
        ${editorCheck("template-action-reveal", "Reveal folder", "Open the file manager", Boolean(actions.reveal), "data-template-action=\"reveal\"")}
        ${editorCheck("template-action-editor", "Open in editor", "Use the configured editor", Boolean(actions.open_in_editor), "data-template-action=\"open_in_editor\"")}
        ${editorCheck("template-action-path", "Print path", "Keep CLI output script-friendly", Boolean(actions.print_path), "data-template-action=\"print_path\"")}
      </div>
      ${editorField("Custom commands", "One shell command per line; {path} is supported", `<textarea class="textarea mono" id="template-commands" rows="6" ${override ? "" : "disabled"}>${esc((actions.commands || []).join("\n"))}</textarea>`, false)}
    </div>`;
}

function editorEmpty(iconName, title, description) {
  return `<div class="editor-empty"><div>${icon(iconName)}</div><strong>${title}</strong><p>${description}</p></div>`;
}

function flattenFolderPaths(nodes = [], parent = "") {
  const paths = [];
  for (const node of nodes) {
    const path = parent ? `${parent}/${node.name}` : node.name;
    paths.push(path);
    paths.push(...flattenFolderPaths(node.children || [], path));
  }
  return paths;
}

function parseFolderPaths(value) {
  const roots = [];
  for (const line of value.split(/\r?\n/).map((item) => item.trim()).filter(Boolean)) {
    const parts = line.split("/").map((item) => item.trim()).filter(Boolean);
    let level = roots;
    for (const part of parts) {
      let node = level.find((item) => item.name === part);
      if (!node) {
        node = { name: part, children: [] };
        level.push(node);
      }
      level = node.children;
    }
  }
  return roots;
}

function lightTreeMarkup(nodes = []) {
  return nodes.map((node) => `<div class="light-tree-node"><span>${icon("folder")}${esc(node.name)}</span>${node.children?.length ? `<div>${lightTreeMarkup(node.children)}</div>` : ""}</div>`).join("");
}

function projectsPage() {
  const projects = state.searchResults ?? state.data.projects;
  return `
    <section class="page">
      <div class="page-header">
        <div><h1 class="page-title">Projects</h1><p class="page-subtitle">Every Fast Folder project in one searchable library, including its ID, template, location, and availability.</p></div>
        <div class="page-actions">
          <button class="button button-secondary" data-view="register">${icon("folderPlus")} Add existing</button>
          <button class="button button-primary" data-start-create>${icon("plus")} New project</button>
        </div>
      </div>
      <div class="search-bar">
        ${icon("search")}
        <input id="project-search" value="${esc(state.search)}" placeholder="Search projects — try tag:draft, template=music-video, created>2026-01-01" autocomplete="off">
      </div>
      <div class="search-legend">
        <span><b>tag:</b>draft</span><span><b>template=</b>music-video</span><span><b>artist=</b>Aria*</span><span><b>created&gt;</b>2026-01-01</span><span>or any free text</span>
      </div>
      ${bulkBar(projects)}
      <div class="panel project-list">
        <div class="project-row project-header-row">
          <label class="project-select"><input type="checkbox" data-select-all ${allSelected(projects) ? "checked" : ""}></label>
          <div class="eyebrow">Project</div><div class="eyebrow">Base</div><div class="eyebrow">Template</div><div class="eyebrow">Created</div><div class="eyebrow">Status</div><span></span>
        </div>
        <div id="project-results">${projectRows(projects)}</div>
      </div>
    </section>`;
}

function allSelected(projects) {
  return projects.length > 0 && projects.every((p) => state.selected.has(p.path));
}

// Bulk-action toolbar shown when ≥1 project is checked: pick a target base and
// move every selected project into it (skipping any already there).
function bulkBar(projects) {
  const count = state.selected.size;
  if (!count) return "";
  const bases = effectiveBases();
  if (bases.length < 2) {
    return `<div class="bulk-bar"><span class="bulk-count">${count} selected</span><span class="bulk-spacer"></span><button class="button button-secondary" data-bulk-clear>Clear</button></div>`;
  }
  const selectedBase = state.bulkBase || bases[0];
  return `
    <div class="bulk-bar">
      <span class="bulk-count">${count} selected</span>
      <span class="bulk-spacer"></span>
      <label class="field-hint" for="bulk-base-select">Move to</label>
      <select class="input mono" id="bulk-base-select" ${state.moving ? "disabled" : ""}>
        ${bases.map((base) => `<option value="${esc(base)}" ${base === selectedBase ? "selected" : ""}>${esc(baseLabel(base))} — ${esc(base)}</option>`).join("")}
      </select>
      <button class="button button-dark" data-bulk-move ${state.moving ? "disabled" : ""}>${icon("external")} ${state.moving ? "Moving…" : `Move ${count}`}</button>
      <button class="button button-secondary" data-bulk-clear ${state.moving ? "disabled" : ""}>Clear</button>
    </div>`;
}

function settingsPage() {
  const config = state.data.config;
  return `
    <section class="page">
      <div class="page-header"><div><h1 class="page-title">Settings</h1><p class="page-subtitle">Control where projects go and how Fast Folder behaves when it creates them.</p></div></div>
      <div class="settings-layout">
        <div class="settings-nav">
          ${settingsNavButton("general", "settings", "General")}
          ${settingsNavButton("data", "database", "Project data")}
          ${settingsNavButton("appearance", "palette", "Appearance")}
        </div>
        ${settingsTabContent(config)}
      </div>
    </section>`;
}

function settingsNavButton(tab, iconName, label) {
  return `<button class="settings-nav-item ${state.settingsTab === tab ? "active" : ""}" data-settings-tab="${tab}">${icon(iconName)} ${label}</button>`;
}

function settingsTabContent(config) {
  if (state.settingsTab === "data") return projectDataSettings(config);
  if (state.settingsTab === "appearance") return appearanceSettings();
  return generalSettings(config);
}

function generalSettings(config) {
  return `
    <form id="general-settings-form" class="settings-panel">
      <div class="settings-section">
        <h3>Project defaults</h3>
        <p>These values become the starting point for every new project. You can still override the location during creation.</p>
        ${settingsInput("settings-base-dir", "Base directory", "Where new projects are created", config.base_dir, "mono")}
        ${settingsSelect("settings-default-template", "Default template", "Preselect a template when creating", config.default_template, [{value:"", label:"Always ask"}, ...state.data.templates.map((template) => ({value: template.slug, label: template.name}))])}
        ${settingsInput("settings-date-format", "Date format", "strftime pattern used in names", config.date_format, "mono")}
      </div>
      <div class="settings-section">
        <h3>Workflow</h3>
        <p>Choose the conveniences Fast Folder should apply to newly created projects.</p>
        ${settingsToggle("settings-git", "Initialize Git", "Start a repository automatically", config.post_create.git_init)}
        ${settingsToggle("settings-open", "Open after creation", "Reveal the new folder when it is ready", config.prompt_open_after_create)}
        ${settingsToggle("settings-confirm", "Confirm before creating", "Ask for confirmation on the CLI before writing", config.confirm_create)}
        ${settingsToggle("settings-banner", "Show CLI banner", "Display the ASCII banner in the terminal TUI", config.show_banner)}
      </div>
      <div class="settings-section">
        <h3>External editor</h3>
        <p>Set the editor Fast Folder should use when a workflow asks to open project files.</p>
        ${settingsInput("settings-editor", "Editor command", "Blank uses your system default", config.editor)}
      </div>
      <div class="settings-footer"><button class="button button-primary" type="submit">${icon("check")} Save general settings</button></div>
    </form>`;
}

function projectDataSettings(config) {
  return `
    <form id="data-settings-form" class="settings-panel">
      <div class="settings-section">
        <h3>Library bases</h3>
        <p>Fast Folder discovers projects from the <code>PROJECT_INFO.md</code> in each folder. Your base directory is always indexed; list any extra folders here (one path per line) to include them too.</p>
        <div class="settings-row">
          <div class="settings-label"><strong>Extra base directories</strong><span>One absolute path per line. Missing folders are skipped.</span></div>
          <textarea id="settings-bases" class="input mono" rows="3" placeholder="/mnt/proj/01_PROJECTS">${esc((config.bases || []).join("\n"))}</textarea>
        </div>
      </div>
      <div class="settings-section">
        <h3>Library behavior</h3>
        <p>Control how much information Fast Folder shows while browsing and previewing projects.</p>
        ${settingsInput("settings-recent", "Recent project limit", "Number of projects shown by default", config.recent_default_limit, "", "number")}
        ${settingsInput("settings-preview-lines", "File preview lines", "Lines shown for generated file previews", config.preview_lines, "", "number")}
      </div>
      <div class="settings-section">
        <h3>Fast Folder data</h3>
        <p>Your current installation is portable. These files contain its templates, configuration, and counter. Projects live in your bases, each with a disposable <code>.fastf-index.json</code> cache.</p>
        <div class="data-summary">
          ${dataSummary("Projects found", state.data.projects.length)}
          ${dataSummary("Templates", state.data.templates.length)}
          ${dataSummary("Current counter", state.data.counter)}
        </div>
        <div class="data-location"><span>Installation</span><strong title="${esc(state.data.install_dir)}">${esc(shortPath(state.data.install_dir))}</strong></div>
        <div class="data-actions">
          <button class="button button-secondary" type="button" data-open-path="${esc(state.data.install_dir)}">${icon("external")} Open data folder</button>
          <button class="button button-secondary" type="button" data-open-path="${esc(state.data.templates_dir)}">${icon("layers")} Open templates</button>
          <button class="button button-secondary" type="button" data-reindex>${icon("refresh")} Reindex library</button>
        </div>
      </div>
      <div class="settings-section">
        <h3>ID counter</h3>
        <p>The global counter shared by every template. The next project will be <strong>${esc(state.data.next_id)}</strong>. Lower it carefully — reusing an ID can collide with existing folders.</p>
        <div class="settings-row">
          <div class="settings-label"><strong>Counter value</strong><span>Last used number; next project is this + 1</span></div>
          <div class="inline-set"><input id="settings-counter" class="input" type="number" min="0" value="${esc(state.data.counter)}"><button class="button button-secondary" type="button" data-save-counter>${icon("check")} Set</button></div>
        </div>
      </div>
      <div class="settings-footer"><button class="button button-primary" type="submit">${icon("check")} Save data settings</button></div>
    </form>`;
}

function dataSummary(label, value) {
  return `<div class="data-summary-card"><strong>${esc(value)}</strong><span>${label}</span></div>`;
}

function appearanceSettings() {
  const appearance = state.appearance;
  return `
    <form id="appearance-settings-form" class="settings-panel">
      <div class="settings-section">
        <h3>Color theme</h3>
        <p>Match your desktop automatically or keep Fast Folder in a fixed light or dark theme.</p>
        ${settingsSelect("appearance-theme", "Theme", "Applied immediately to this app", appearance.theme, [
          { value: "system", label: "Use system setting" },
          { value: "light", label: "Light" },
          { value: "dark", label: "Dark" },
        ])}
      </div>
      <div class="settings-section">
        <h3>Accent color</h3>
        <p>Choose the highlight used for primary actions, selected items, and status indicators.</p>
        <div class="appearance-choice-grid">
          ${accentChoice("lime", "Electric lime", "#c9f560")}
          ${accentChoice("blue", "Clear blue", "#8fb7ff")}
          ${accentChoice("violet", "Soft violet", "#bca7ff")}
        </div>
      </div>
      <div class="settings-section">
        <h3>Interface density</h3>
        <p>Comfortable spacing is easier to scan. Compact spacing keeps more projects and templates visible.</p>
        <div class="segmented-control">
          ${densityChoice("comfortable", "Comfortable")}
          ${densityChoice("compact", "Compact")}
        </div>
      </div>
      <div class="settings-section">
        <h3>Preview</h3>
        <p>The choices above are applied live. This sample shows the current visual language.</p>
        <div class="appearance-preview">
          <div class="appearance-preview-icon">${icon("folder")}</div>
          <div><strong>Fast Folder project</strong><span>${esc(state.data.next_id)} · Ready to create</span></div>
          <span class="button button-primary">${icon("bolt")} Create</span>
        </div>
      </div>
      <div class="settings-footer"><button class="button button-primary" type="submit">${icon("check")} Save appearance</button></div>
    </form>`;
}

function accentChoice(value, label, color) {
  return `<button class="appearance-choice ${state.appearance.accent === value ? "active" : ""}" type="button" data-accent-choice="${value}"><i style="background:${color}"></i><span>${label}</span>${state.appearance.accent === value ? icon("check") : ""}</button>`;
}

function densityChoice(value, label) {
  return `<button class="${state.appearance.density === value ? "active" : ""}" type="button" data-density-choice="${value}">${label}</button>`;
}

function settingsInput(id, label, hint, value, className = "", type = "text") {
  return `<div class="settings-row"><div class="settings-label"><strong>${label}</strong><span>${hint}</span></div><input id="${id}" class="input ${className}" type="${type}" ${type === "number" ? 'min="0"' : ""} value="${esc(value)}"></div>`;
}

function settingsSelect(id, label, hint, value, options) {
  return `<div class="settings-row"><div class="settings-label"><strong>${label}</strong><span>${hint}</span></div><select id="${id}" class="select">${options.map((option) => `<option value="${esc(option.value)}" ${value === option.value ? "selected" : ""}>${esc(option.label)}</option>`).join("")}</select></div>`;
}

function settingsToggle(id, label, hint, checked) {
  return `<div class="settings-row"><div class="settings-label"><strong>${label}</strong><span>${hint}</span></div><label class="switch"><input id="${id}" type="checkbox" ${checked ? "checked" : ""}><span></span></label></div>`;
}

function filterTemplates() {
  const query = state.search.trim().toLowerCase();
  if (!query) return state.data.templates;
  return state.data.templates.filter((template) => `${template.name} ${template.slug} ${template.description}`.toLowerCase().includes(query));
}

function emptySearch(message) {
  return `<div class="empty"><div class="empty-icon">${icon("search")}</div><strong>${esc(message)}</strong><p>Try a different project name, template, ID, or tag.</p></div>`;
}

function bindCommon() {
  document.querySelectorAll("[data-view]").forEach((button) => button.addEventListener("click", () => {
    state.view = button.dataset.view;
    render();
  }));
  document.querySelectorAll("[data-start-create]").forEach((button) => button.addEventListener("click", () => {
    state.view = "create";
    render();
  }));
  document.querySelectorAll("[data-template]").forEach((button) => button.addEventListener("click", () => selectTemplate(button.dataset.template)));
  document.querySelectorAll("[data-edit-template]").forEach((button) => button.addEventListener("click", () => openTemplateEditor(button.dataset.editTemplate)));
  document.querySelector("[data-new-template]")?.addEventListener("click", () => openTemplateEditor());
  document.querySelectorAll("[data-open-path]").forEach((button) => button.addEventListener("click", (event) => {
    event.stopPropagation();
    openPath(button.dataset.openPath);
  }));
  document.querySelector('[data-action="refresh"]')?.addEventListener("click", refresh);
  document.querySelector("[data-from-folder]")?.addEventListener("click", openFromFolderModal);
  document.querySelector("[data-reconcile]")?.addEventListener("click", reconcileProvisioning);
  bindProjectRows();
  bindProjectBulk();

  const search = document.querySelector("#global-search");
  search?.addEventListener("input", (event) => {
    state.search = event.target.value;
    if (state.view === "templates") {
      render();
      refocusGlobalSearch();
      return;
    }
    if (state.view !== "projects") {
      state.view = "projects";
      state.searchResults = null;
      render();
      refocusGlobalSearch();
    }
    scheduleProjectSearch();
  });

  document.querySelector("#project-search")?.addEventListener("input", (event) => {
    state.search = event.target.value;
    scheduleProjectSearch();
  });
}

function refocusGlobalSearch() {
  requestAnimationFrame(() => {
    const current = document.querySelector("#global-search");
    current?.focus();
    current?.setSelectionRange(current.value.length, current.value.length);
  });
}

function bindProjectRows() {
  document.querySelectorAll("[data-detail-path]").forEach((row) => {
    row.addEventListener("click", (event) => {
      if (event.target.closest("[data-open-path]") || event.target.closest(".project-select")) return;
      openProjectDetail(row.dataset.detailPath);
    });
    row.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        openProjectDetail(row.dataset.detailPath);
      }
    });
  });
  document.querySelectorAll("[data-select-path]").forEach((box) => {
    box.addEventListener("click", (event) => event.stopPropagation());
    box.addEventListener("change", () => {
      const path = box.dataset.selectPath;
      if (box.checked) state.selected.add(path);
      else state.selected.delete(path);
      render();
    });
  });
}

// Bulk-select toolbar (page-level, outside #project-results). Bound on full
// render only, so in-place search updates don't double-bind it.
function bindProjectBulk() {
  document.querySelector("[data-select-all]")?.addEventListener("change", (event) => {
    const projects = state.searchResults ?? state.data.projects;
    if (event.target.checked) projects.forEach((p) => state.selected.add(p.path));
    else projects.forEach((p) => state.selected.delete(p.path));
    render();
  });
  document.querySelector("#bulk-base-select")?.addEventListener("change", (event) => {
    state.bulkBase = event.target.value;
  });
  document.querySelector("[data-bulk-clear]")?.addEventListener("click", () => {
    state.selected.clear();
    render();
  });
  document.querySelector("[data-bulk-move]")?.addEventListener("click", moveSelected);
}

function scheduleProjectSearch() {
  clearTimeout(state.searchTimer);
  state.searchTimer = setTimeout(runProjectSearch, 220);
}

async function runProjectSearch() {
  const terms = state.search.trim().split(/\s+/).filter(Boolean);
  try {
    const result = await api("/api/search", { method: "POST", body: JSON.stringify({ terms }) });
    state.searchResults = result.projects;
  } catch (error) {
    state.searchResults = [];
    toast(error.message, true);
  }
  const node = document.querySelector("#project-results");
  if (node) {
    node.innerHTML = projectRows(state.searchResults);
    bindProjectRows();
    document.querySelectorAll("#project-results [data-open-path]").forEach((button) => button.addEventListener("click", (event) => {
      event.stopPropagation();
      openPath(button.dataset.openPath);
    }));
  }
}

function bindView() {
  if (state.view === "create") bindCreate();
  if (state.view === "template-editor") bindTemplateEditor();
  if (state.view === "register") bindRegister();
  if (state.view === "settings") bindSettings();
}

function selectTemplate(slug) {
  const template = state.data.templates.find((item) => item.slug === slug);
  if (!template) return;
  state.selectedTemplate = slug;
  initializeVariables(template);
  state.view = "create";
  render();
  updatePreview();
}

function bindCreate() {
  document.querySelectorAll("[data-select-template]").forEach((option) => option.addEventListener("click", () => selectTemplate(option.dataset.selectTemplate)));
  document.querySelectorAll("[data-variable]").forEach((field) => {
    const update = (event) => {
      state.variables[event.target.dataset.variable] = event.target.value;
      schedulePreview();
    };
    field.addEventListener("input", update);
    field.addEventListener("change", update);
  });
  document.querySelector("#base-dir")?.addEventListener("change", schedulePreview);
  document.querySelector("#git-init")?.addEventListener("change", (event) => { state.gitInit = event.target.checked; });
  document.querySelector("#reveal-folder")?.addEventListener("change", (event) => { state.reveal = event.target.checked; });
  document.querySelector("#create-form")?.addEventListener("submit", createProject);
}

function bindTemplateEditor() {
  document.querySelectorAll("[data-editor-section]").forEach((button) => button.addEventListener("click", () => {
    state.templateEditorSection = button.dataset.editorSection;
    render();
    if (button.dataset.editorSection === "files" && state.templateFiles === null && state.templateOriginalSlug) {
      loadTemplateFiles(state.templateOriginalSlug);
    }
  }));
  document.querySelector("[data-cancel-template]")?.addEventListener("click", closeTemplateEditor);
  document.querySelector("[data-save-template]")?.addEventListener("click", saveTemplate);
  document.querySelector("[data-delete-template]")?.addEventListener("click", deleteTemplate);

  bindTemplateBasics();
  bindTemplateVariables();
  bindTemplateStructure();
  bindTemplateFiles();
  bindTemplateAutomation();
}

function bindTemplateBasics() {
  const template = state.templateEditor;
  const name = document.querySelector("#template-name");
  const slug = document.querySelector("#template-slug");
  name?.addEventListener("input", (event) => {
    template.name = event.target.value;
    if (!state.templateSlugTouched) {
      template.slug = slugify(event.target.value);
      if (slug) slug.value = template.slug;
    }
    state.templateDirty = true;
    updateTemplateSummary();
  });
  slug?.addEventListener("input", (event) => {
    state.templateSlugTouched = true;
    template.slug = slugify(event.target.value, true);
    event.target.value = template.slug;
    state.templateDirty = true;
    updateTemplateSummary();
  });
  bindValue("#template-description", (value) => { template.description = value; });
  bindValue("#template-pattern", (value) => {
    template.naming_pattern = value;
    updatePatternPreview();
  });
  bindValue("#template-id-prefix", (value) => {
    template.id.prefix = value;
    updatePatternPreview();
  });
  bindValue("#template-id-digits", (value) => {
    template.id.digits = Math.max(1, Math.min(12, Number(value) || 4));
    updatePatternPreview();
  });
}

function bindValue(selector, mutator, eventName = "input") {
  document.querySelector(selector)?.addEventListener(eventName, (event) => {
    mutator(event.target.value);
    state.templateDirty = true;
  });
}

function updateTemplateSummary() {
  const summary = document.querySelector(".editor-summary");
  if (!summary) return;
  const strong = summary.querySelector("strong");
  const span = summary.querySelector("span");
  if (strong) strong.textContent = state.templateEditor.name || "Untitled template";
  if (span) span.textContent = state.templateEditor.slug || "no-slug";
}

function updatePatternPreview() {
  const preview = document.querySelector(".pattern-preview strong");
  if (preview) preview.textContent = templatePatternExample(state.templateEditor);
}

function slugify(value, preserveUnderscore = false) {
  const separator = preserveUnderscore ? /[^a-zA-Z0-9_-]+/g : /[^a-zA-Z0-9]+/g;
  return value
    .trim()
    .replace(separator, "-")
    .replace(/^-+|-+$/g, "")
    .toLowerCase();
}

function bindTemplateVariables() {
  const template = state.templateEditor;
  document.querySelector("[data-add-variable]")?.addEventListener("click", () => {
    let number = template.variables.length + 1;
    let slug = `field_${number}`;
    while (template.variables.some((variable) => variable.slug === slug)) {
      number += 1;
      slug = `field_${number}`;
    }
    template.variables.push({
      slug,
      label: `Question ${number}`,
      type: "text",
      required: false,
      options: [],
      default: "",
      transform: "none",
    });
    state.templateDirty = true;
    render();
  });
  document.querySelectorAll("[data-remove-variable]").forEach((button) => button.addEventListener("click", () => {
    const index = Number(button.dataset.removeVariable);
    const [removed] = template.variables.splice(index, 1);
    template.tag_from = (template.tag_from || []).filter((slug) => slug !== removed?.slug);
    state.templateDirty = true;
    render();
  }));
  document.querySelectorAll("[data-variable-field]").forEach((field) => field.addEventListener("change", updateTemplateVariable));
  document.querySelectorAll("[data-variable-field]").forEach((field) => {
    if (field.tagName !== "SELECT") field.addEventListener("input", updateTemplateVariable);
  });
  document.querySelectorAll("[data-variable-required]").forEach((field) => field.addEventListener("change", () => {
    template.variables[Number(field.dataset.variableRequired)].required = field.checked;
    state.templateDirty = true;
  }));
  document.querySelectorAll("[data-variable-tag]").forEach((field) => field.addEventListener("change", () => {
    const variable = template.variables[Number(field.dataset.variableTag)];
    const tags = new Set(template.tag_from || []);
    if (field.checked) tags.add(variable.slug);
    else tags.delete(variable.slug);
    template.tag_from = [...tags];
    state.templateDirty = true;
  }));
}

function updateTemplateVariable(event) {
  const template = state.templateEditor;
  const index = Number(event.target.dataset.variableIndex);
  const field = event.target.dataset.variableField;
  const variable = template.variables[index];
  if (!variable) return;
  if (field === "options") {
    variable.options = commaList(event.target.value);
  } else if (field === "slug") {
    const oldSlug = variable.slug;
    variable.slug = slugify(event.target.value, true).replaceAll("-", "_");
    event.target.value = variable.slug;
    template.tag_from = (template.tag_from || []).map((slug) => slug === oldSlug ? variable.slug : slug);
  } else {
    variable[field] = event.target.value;
  }
  state.templateDirty = true;
  if (field === "type") render();
}

function commaList(value) {
  return [...new Set(value.split(",").map((item) => item.trim()).filter(Boolean))];
}

function bindTemplateStructure() {
  document.querySelector("#template-folders")?.addEventListener("input", (event) => {
    state.templateEditor.structure = parseFolderPaths(event.target.value);
    state.templateDirty = true;
    const preview = document.querySelector("#template-structure-preview");
    if (preview) {
      preview.innerHTML = state.templateEditor.structure.length
        ? lightTreeMarkup(state.templateEditor.structure)
        : `<span class="field-hint">Add folder paths to see the tree.</span>`;
    }
  });
}

// Files are stored on disk in the template's files/ folder, so every operation
// hits its own endpoint and re-reads the list — independent of the metadata
// "Save template" button.
function bindTemplateFiles() {
  document.querySelector("[data-file-new]")?.addEventListener("click", createTemplateFile);
  document.querySelector("[data-file-add-path]")?.addEventListener("click", addTemplateFileFromPath);
  document.querySelectorAll("[data-file-delete]").forEach((button) =>
    button.addEventListener("click", () => deleteTemplateFile(button.dataset.fileDelete)));
  document.querySelectorAll("[data-file-path]").forEach((area) =>
    area.addEventListener("blur", () => saveTemplateFileContent(area.dataset.filePath, area.value)));
}

async function createTemplateFile() {
  const input = document.querySelector("#file-new-path");
  const path = input?.value.trim();
  if (!path) {
    toast("Enter a path for the new file.", true);
    return;
  }
  await runFileOp("/api/templates/file-save", { slug: state.templateOriginalSlug, path, content: "" }, `Created ${path}.`);
}

async function addTemplateFileFromPath() {
  const src = document.querySelector("#file-src")?.value.trim();
  const destInput = document.querySelector("#file-dest")?.value.trim();
  if (!src) {
    toast("Enter a source file path to copy from.", true);
    return;
  }
  const dest = destInput || src.split("/").pop();
  await runFileOp("/api/templates/file-add", { slug: state.templateOriginalSlug, src, dest }, `Added ${dest}.`);
}

async function deleteTemplateFile(path) {
  if (!window.confirm(`Remove ${path} from this template?`)) return;
  await runFileOp("/api/templates/file-delete", { slug: state.templateOriginalSlug, path }, `Removed ${path}.`);
}

async function saveTemplateFileContent(path, content) {
  const existing = (state.templateFiles || []).find((file) => file.path === path);
  if (existing && existing.content === content) return; // no-op — unchanged
  try {
    await api("/api/templates/file-save", { method: "POST", body: JSON.stringify({ slug: state.templateOriginalSlug, path, content }) });
    if (existing) existing.content = content;
    toast(`Saved ${path}.`);
  } catch (error) {
    toast(error.message, true);
  }
}

async function runFileOp(route, payload, message) {
  try {
    await api(route, { method: "POST", body: JSON.stringify(payload) });
    toast(message);
    await loadTemplateFiles(state.templateOriginalSlug); // re-reads + re-renders
  } catch (error) {
    toast(error.message, true);
  }
}

function bindTemplateAutomation() {
  const template = state.templateEditor;
  bindValue("#template-tags", (value) => { template.tags = commaList(value); });
  bindValue("#template-verbatim", (value) => { template.verbatim = commaList(value); });
  bindValue("#template-exclude", (value) => { template.exclude = commaList(value); });
  document.querySelector("[data-template-override-actions]")?.addEventListener("change", (event) => {
    template.post_create = event.target.checked
      ? { git_init: false, reveal: false, open_in_editor: false, print_path: false, commands: [] }
      : null;
    state.templateDirty = true;
    render();
  });
  document.querySelectorAll("[data-template-action]").forEach((field) => field.addEventListener("change", () => {
    if (!template.post_create) return;
    template.post_create[field.dataset.templateAction] = field.checked;
    state.templateDirty = true;
  }));
  bindValue("#template-commands", (value) => {
    if (template.post_create) {
      template.post_create.commands = value.split(/\r?\n/).map((item) => item.trim()).filter(Boolean);
    }
  });
}

function closeTemplateEditor() {
  if (state.templateDirty && !window.confirm("Discard the unsaved template changes?")) return;
  state.templateEditor = null;
  state.templateOriginalSlug = null;
  state.templateDirty = false;
  state.view = "templates";
  render();
}

async function saveTemplate() {
  if (state.savingTemplate) return;
  state.savingTemplate = true;
  const button = document.querySelector("[data-save-template]");
  if (button) {
    button.disabled = true;
    button.textContent = "Saving…";
  }
  try {
    const result = await api("/api/templates/save", {
      method: "POST",
      body: JSON.stringify({
        original_slug: state.templateOriginalSlug,
        template: state.templateEditor,
      }),
    });
    await loadState(false);
    state.templateEditor = null;
    state.templateOriginalSlug = null;
    state.templateDirty = false;
    state.view = "templates";
    render();
    toast(`Template “${result.template.name}” saved.`);
  } catch (error) {
    toast(error.message, true);
    if (button) {
      button.disabled = false;
      button.innerHTML = `${icon("check")} Save template`;
    }
  } finally {
    state.savingTemplate = false;
  }
}

async function deleteTemplate() {
  const slug = state.templateOriginalSlug;
  if (!slug || !window.confirm(`Delete the template “${state.templateEditor.name}”? Projects already created from it will not be removed.`)) return;
  try {
    await api("/api/templates/delete", {
      method: "POST",
      body: JSON.stringify({ slug }),
    });
    await loadState(false);
    state.templateEditor = null;
    state.templateOriginalSlug = null;
    state.templateDirty = false;
    state.view = "templates";
    render();
    toast("Template deleted.");
  } catch (error) {
    toast(error.message, true);
  }
}

function schedulePreview() {
  clearTimeout(state.previewTimer);
  state.previewTimer = setTimeout(updatePreview, 220);
}

async function updatePreview() {
  if (state.view !== "create" || !state.selectedTemplate) return;
  const baseDir = document.querySelector("#base-dir")?.value ?? state.data.config.base_dir;
  try {
    state.preview = await api("/api/preview", {
      method: "POST",
      body: JSON.stringify({ template: state.selectedTemplate, variables: state.variables, base_dir: baseDir }),
    });
    state.previewError = "";
  } catch (error) {
    state.preview = null;
    state.previewError = error.message;
  }
  const previewPane = document.querySelector(".preview-pane");
  if (previewPane) previewPane.innerHTML = previewPanel();
  const errorBox = document.querySelector("#create-error");
  if (errorBox) errorBox.innerHTML = state.previewError ? `<div class="error-box" style="margin-top:14px">${esc(state.previewError)}</div>` : "";
}

async function createProject(event) {
  event.preventDefault();
  if (state.creating) return;
  state.gitInit = document.querySelector("#git-init")?.checked ?? state.gitInit;
  state.reveal = document.querySelector("#reveal-folder")?.checked ?? state.reveal;
  const baseDir = document.querySelector("#base-dir")?.value ?? state.data.config.base_dir;
  state.creating = true;
  const submit = event.currentTarget.querySelector('[type="submit"]');
  if (submit) { submit.disabled = true; submit.textContent = "Creating…"; }
  try {
    const result = await api("/api/create", {
      method: "POST",
      body: JSON.stringify({
        template: state.selectedTemplate,
        variables: state.variables,
        base_dir: baseDir,
        git_init: state.gitInit,
        reveal: state.reveal,
      }),
    });
    showSuccess(result);
    await loadState(false);
  } catch (error) {
    state.previewError = error.message;
    toast(error.message, true);
    const errorBox = document.querySelector("#create-error");
    if (errorBox) errorBox.innerHTML = `<div class="error-box" style="margin-top:14px">${esc(error.message)}</div>`;
  } finally {
    state.creating = false;
    if (submit) { submit.disabled = false; submit.innerHTML = `${icon("bolt")} Create project`; }
  }
}

function showSuccess(result) {
  const project = result.project || result;
  const jobId = result.job_id || null;
  const progressMarkup = jobId
    ? `<div class="job-progress" id="job-progress">
         <div class="job-bar"><div class="job-bar-fill" style="width:0%"></div></div>
         <div class="job-label">Copying bundled files…</div>
       </div>`
    : "";
  const overlay = document.createElement("div");
  overlay.className = "success-overlay";
  overlay.innerHTML = `
    <div class="success-modal">
      <div class="success-mark">${icon("check")}</div>
      <h2>Project created.</h2>
      <p>Fast Folder created the complete <strong>${esc(project.template_name)}</strong> system with project ID <strong>${esc(project.id)}</strong>.</p>
      <div class="success-path" title="${esc(project.root_path)}">${esc(project.root_path)}</div>
      ${progressMarkup}
      <div class="success-actions">
        <button class="button button-secondary" data-close-success>Done</button>
        <button class="button button-dark" data-success-open>${icon("external")} Open folder</button>
      </div>
    </div>`;
  document.body.append(overlay);
  overlay.querySelector("[data-close-success]").addEventListener("click", () => {
    overlay.remove();
    state.view = "dashboard";
    render();
  });
  // "Open folder" works immediately — the structure + small files already exist.
  overlay.querySelector("[data-success-open]").addEventListener("click", () => openPath(project.root_path));
  if (jobId) pollJob(jobId, overlay);
}

// Poll a background asset-copy job, updating only the progress node inside the
// success overlay (no full re-render). A 404 means the job was evicted after
// finishing — treat it as done.
async function pollJob(jobId, overlay) {
  const node = overlay.querySelector("#job-progress");
  if (!node) return;
  const fill = () => node.querySelector(".job-bar-fill");
  const label = () => node.querySelector(".job-label");

  while (document.body.contains(overlay)) {
    let job;
    try {
      const res = await api(`/api/job/${encodeURIComponent(jobId)}`);
      job = res.job;
    } catch {
      break; // evicted → complete
    }
    const pct = job.total_bytes
      ? Math.min(100, Math.round((job.copied_bytes / job.total_bytes) * 100))
      : 100;
    if (fill()) fill().style.width = pct + "%";
    if (job.status === "failed") {
      node.classList.add("job-failed");
      if (label()) label().textContent = `Copy failed: ${job.error || "unknown error"}`;
      toast("Asset copy failed.", true);
      return;
    }
    if (job.status === "done") break;
    if (label()) {
      label().textContent = `Copying ${job.current_file || "files"}… ${pct}% (${job.done_files}/${job.total_files})`;
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }

  if (!document.body.contains(overlay)) return;
  node.classList.add("job-done");
  if (fill()) fill().style.width = "100%";
  if (label()) label().textContent = "Bundled files copied.";
  toast("Bundled files copied.");
}

async function openPath(path) {
  try {
    await api("/api/open", { method: "POST", body: JSON.stringify({ path }) });
  } catch (error) {
    toast(error.message, true);
  }
}

function bindSettings() {
  document.querySelectorAll("[data-settings-tab]").forEach((button) => button.addEventListener("click", () => {
    state.settingsTab = button.dataset.settingsTab;
    render();
  }));

  document.querySelector("#general-settings-form")?.addEventListener("submit", async (event) => {
    event.preventDefault();
    const payload = {
      base_dir: document.querySelector("#settings-base-dir").value,
      editor: document.querySelector("#settings-editor").value,
      default_template: document.querySelector("#settings-default-template").value,
      date_format: document.querySelector("#settings-date-format").value,
      prompt_open_after_create: document.querySelector("#settings-open").checked,
      git_init: document.querySelector("#settings-git").checked,
      confirm_create: document.querySelector("#settings-confirm").checked,
      show_banner: document.querySelector("#settings-banner").checked,
    };
    await saveSettings(payload, "General settings saved.");
  });

  document.querySelector("#data-settings-form")?.addEventListener("submit", async (event) => {
    event.preventDefault();
    const bases = (document.querySelector("#settings-bases").value || "")
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => line.length > 0);
    const payload = {
      bases,
      recent_default_limit: Number(document.querySelector("#settings-recent").value || 20),
      preview_lines: Number(document.querySelector("#settings-preview-lines").value || 0),
    };
    await saveSettings(payload, "Project data settings saved.");
  });

  document.querySelector("[data-reindex]")?.addEventListener("click", async () => {
    try {
      const result = await api("/api/reindex", { method: "POST", body: JSON.stringify({}) });
      await loadState(false);
      state.searchResults = null;
      render();
      toast(`Reindexed ${result.projects} project${result.projects === 1 ? "" : "s"}.`);
    } catch (error) {
      toast(error.message, true);
    }
  });

  document.querySelector("[data-save-counter]")?.addEventListener("click", async () => {
    const value = Number(document.querySelector("#settings-counter").value);
    if (!Number.isFinite(value) || value < 0) {
      toast("Enter a counter value of 0 or more.", true);
      return;
    }
    try {
      await api("/api/counter", { method: "POST", body: JSON.stringify({ value }) });
      await loadState(false);
      render();
      toast(`Counter set — next project is ${state.data.next_id}.`);
    } catch (error) {
      toast(error.message, true);
    }
  });

  document.querySelector("#appearance-theme")?.addEventListener("change", (event) => {
    state.appearance.theme = event.target.value;
    applyAppearance();
  });
  document.querySelectorAll("[data-accent-choice]").forEach((button) => button.addEventListener("click", () => {
    state.appearance.accent = button.dataset.accentChoice;
    applyAppearance();
    render();
  }));
  document.querySelectorAll("[data-density-choice]").forEach((button) => button.addEventListener("click", () => {
    state.appearance.density = button.dataset.densityChoice;
    applyAppearance();
    render();
  }));
  document.querySelector("#appearance-settings-form")?.addEventListener("submit", (event) => {
    event.preventDefault();
    localStorage.setItem("fastf-appearance", JSON.stringify(state.appearance));
    toast("Appearance saved.");
  });
}

async function saveSettings(payload, message) {
  try {
    await api("/api/settings", { method: "POST", body: JSON.stringify(payload) });
    await loadState(false);
    render();
    toast(message);
  } catch (error) {
    toast(error.message, true);
  }
}

// ---------------------------------------------------------------------------
// Register existing folder
// ---------------------------------------------------------------------------

function registerPage() {
  const reg = state.register;
  const templates = state.data.templates;
  const selected = templates.find((template) => template.slug === reg.template);
  return `
    <section class="page">
      <div class="page-header"><div>
        <h1 class="page-title">Add an existing folder</h1>
        <p class="page-subtitle">Onboard a folder Fast Folder didn’t create. It gets a project ID and a metadata file, then appears in search, tags, and notes — just like a created project.</p>
      </div></div>
      <form id="register-form" class="settings-panel register-panel">
        <div class="settings-section">
          <h3>Folder</h3>
          <p>The absolute path to the folder you want to onboard.</p>
          ${editorField("Folder path", "Nothing inside is changed unless you fill structure", `<input class="input mono" id="register-path" value="${esc(reg.path)}" placeholder="/home/you/old-project">`, true, "full")}
        </div>
        <div class="settings-section">
          <h3>Template <span class="optional">optional</span></h3>
          <p>Attaching a template records its variables and tags, and unlocks structure fill-in.</p>
          <select class="select" id="register-template">
            <option value="" ${reg.template === "" ? "selected" : ""}>No template — index only</option>
            ${templates.filter((template) => template.slug !== "(registered)").map((template) => `<option value="${esc(template.slug)}" ${reg.template === template.slug ? "selected" : ""}>${esc(template.name)}</option>`).join("")}
          </select>
          ${selected && selected.variables.length ? `<div class="register-vars">${selected.variables.map(registerVariableField).join("")}</div>` : ""}
        </div>
        <div class="settings-section">
          <h3>Options</h3>
          ${stateToggleRow("register-rename", "Standardize folder name", "Rename to match the naming pattern", reg.rename)}
          ${stateToggleRow("register-apply", "Fill missing structure", selected ? "Create folders and files the template defines" : "Choose a template to enable", reg.apply && Boolean(selected), !selected)}
          ${stateToggleRow("register-today", "Use today as the created date", "Otherwise the folder’s own timestamp is used", reg.useToday)}
          ${editorField("Created date", "Override the recorded date", `<input class="input" type="date" id="register-created" value="${esc(reg.created)}" ${reg.useToday ? "disabled" : ""}>`, false, "full")}
        </div>
        <div class="settings-footer">
          <button class="button button-primary" type="submit" ${reg.busy ? "disabled" : ""}>${reg.busy ? "Registering…" : `${icon("folderPlus")} Register folder`}</button>
        </div>
      </form>
    </section>`;
}

function registerVariableField(variable) {
  const value = state.register.variables?.[variable.slug] ?? variable.default ?? "";
  const type = variable.type ?? variable.var_type ?? "text";
  const field = type === "select"
    ? `<select class="select" data-register-var="${esc(variable.slug)}">${variable.options.map((option) => `<option value="${esc(option)}" ${value === option ? "selected" : ""}>${esc(option)}</option>`).join("")}</select>`
    : `<input class="input" data-register-var="${esc(variable.slug)}" value="${esc(value)}" placeholder="${esc(variable.label)}" autocomplete="off">`;
  return `<div class="field"><div class="field-row"><label>${esc(variable.label)}${variable.required ? '<span class="required">*</span>' : ""}</label></div>${field}</div>`;
}

function stateToggleRow(id, label, hint, checked, disabled = false) {
  return `<div class="settings-row"><div class="settings-label"><strong>${label}</strong><span>${hint}</span></div><label class="switch"><input id="${id}" type="checkbox" ${checked ? "checked" : ""} ${disabled ? "disabled" : ""}><span></span></label></div>`;
}

function bindRegister() {
  const reg = state.register;
  document.querySelector("#register-path")?.addEventListener("input", (event) => { reg.path = event.target.value; });
  document.querySelector("#register-template")?.addEventListener("change", (event) => {
    reg.template = event.target.value;
    const template = state.data.templates.find((item) => item.slug === reg.template);
    reg.variables = template ? Object.fromEntries(template.variables.map((variable) => [variable.slug, variable.default || ""])) : {};
    if (!template) reg.apply = false;
    render();
  });
  document.querySelectorAll("[data-register-var]").forEach((field) => {
    const update = (event) => { reg.variables[event.target.dataset.registerVar] = event.target.value; };
    field.addEventListener("input", update);
    field.addEventListener("change", update);
  });
  document.querySelector("#register-rename")?.addEventListener("change", (event) => { reg.rename = event.target.checked; });
  document.querySelector("#register-apply")?.addEventListener("change", (event) => { reg.apply = event.target.checked; });
  document.querySelector("#register-today")?.addEventListener("change", (event) => { reg.useToday = event.target.checked; render(); });
  document.querySelector("#register-created")?.addEventListener("input", (event) => { reg.created = event.target.value; });
  document.querySelector("#register-form")?.addEventListener("submit", submitRegister);
}

async function submitRegister(event) {
  event.preventDefault();
  const reg = state.register;
  reg.path = document.querySelector("#register-path")?.value.trim() || "";
  if (!reg.path) {
    toast("Enter a folder path to register.", true);
    return;
  }
  reg.busy = true;
  render();
  await doRegister(false);
}

async function doRegister(overwrite) {
  const reg = state.register;
  const payload = {
    path: reg.path,
    template: reg.template || null,
    variables: reg.variables || {},
    rename: reg.rename,
    apply: reg.apply && Boolean(reg.template),
    use_today: reg.useToday,
    created: reg.useToday ? null : (reg.created || null),
    overwrite,
  };
  try {
    const result = await api("/api/register", { method: "POST", body: JSON.stringify(payload) });
    await loadState(false);
    const path = result.project.path;
    state.register = { path: "", template: "", variables: {}, rename: false, apply: false, useToday: false, created: "", busy: false };
    state.view = "projects";
    state.searchResults = null;
    render();
    toast(`Registered ${result.project.id}${result.project.renamed_to ? ` as ${result.project.renamed_to}` : ""}.`);
    openProjectDetail(path);
  } catch (error) {
    reg.busy = false;
    if (/already exists/i.test(error.message) && !overwrite) {
      if (window.confirm(`${error.message}\n\nOverwrite the existing metadata file and register anyway?`)) {
        reg.busy = true;
        render();
        await doRegister(true);
        return;
      }
      render();
      return;
    }
    toast(error.message, true);
    render();
  }
}

// ---------------------------------------------------------------------------
// Project detail drawer
// ---------------------------------------------------------------------------

async function openProjectDetail(path) {
  state.detail = { path, data: null, error: "" };
  render();
  try {
    state.detail.data = await api(`/api/project?path=${encodeURIComponent(path)}`);
  } catch (error) {
    state.detail.error = error.message;
  }
  render();
}

function closeDetail() {
  state.detail = null;
  render();
}

function drawerMarkup() {
  if (!state.detail) return "";
  const detail = state.detail;
  let body;
  if (!detail.data) {
    body = detail.error
      ? `<div class="error-box">${esc(detail.error)}</div>`
      : `<div class="drawer-loading">${icon("refresh")} Loading…</div>`;
  } else {
    body = detailBody(detail.data);
  }
  return `
    <div class="drawer-scrim" data-close-detail></div>
    <aside class="drawer" role="dialog" aria-modal="true">
      <div class="drawer-head"><span class="eyebrow">Project</span><button class="icon-button" data-close-detail title="Close">${icon("close")}</button></div>
      <div class="drawer-body scroll">${body}</div>
    </aside>`;
}

function detailBody(data) {
  const meta = data.metadata;
  const record = data.record;
  const name = meta?.folder || record?.name || data.path.split("/").pop();
  const id = meta?.id || record?.id || "—";
  const templateLabel = meta?.template_name || templateName(record?.template || "") || "—";
  const created = meta?.created || record?.created_at;
  const tags = meta?.tags || [];
  const vars = meta?.variables || {};
  const varKeys = Object.keys(vars);
  return `
    <div class="drawer-title">
      <div class="project-folder ${data.exists ? "" : "missing"}">${icon("folder")}</div>
      <div class="drawer-title-copy">
        <h2 title="${esc(name)}">${esc(name)}</h2>
        <div class="drawer-sub"><span class="chip">${esc(id)}</span><span>${esc(templateLabel)}</span><span class="status ${data.exists ? "" : "missing"}">${data.exists ? "Available" : "Missing"}</span></div>
      </div>
    </div>
    <div class="drawer-path" title="${esc(data.path)}">${esc(shortPath(data.path))}</div>
    ${created ? `<div class="drawer-meta-line"><span>Created</span><strong>${esc(formatDate(created))}</strong></div>` : ""}
    ${record?.base ? `<div class="drawer-meta-line"><span>Base</span><strong title="${esc(record.base)}">${esc(record.base_label || record.base)}</strong></div>` : ""}
    ${moveControls(record)}
    <div class="drawer-actions">
      <button class="button button-secondary" data-detail-open ${data.exists ? "" : "disabled"}>${icon("external")} Open</button>
      <button class="button button-secondary" data-detail-apply>${icon("bolt")} Apply template</button>
      <button class="button button-secondary" data-detail-copy>${icon("copy")} Copy path</button>
    </div>
    ${!meta ? `<div class="drawer-empty">${icon("info")}<p>No metadata file found. This folder predates Fast Folder metadata or was hand-edited. Register it or apply a template to add one.</p></div>` : `
      <div class="drawer-section">
        <div class="drawer-section-head">${icon("tag")}<h3>Tags</h3></div>
        <div class="chip-set">
          ${tags.length ? tags.map((tag) => `<span class="chip removable">${esc(tag)}<button data-remove-tag="${esc(tag)}" title="Remove tag">${icon("close")}</button></span>`).join("") : `<span class="field-hint">No tags yet</span>`}
        </div>
        <form class="chip-add" id="add-tag-form"><input class="input" id="add-tag-input" placeholder="Add a tag…" autocomplete="off"><button class="button button-secondary" type="submit" title="Add tag">${icon("plus")}</button></form>
      </div>
      ${varKeys.length ? `<div class="drawer-section">
        <div class="drawer-section-head">${icon("sliders")}<h3>Variables</h3></div>
        <div class="detail-vars">${varKeys.map((key) => `<div class="detail-var"><span>${esc(key)}</span><strong>${vars[key] ? esc(vars[key]) : '<i class="muted">(empty)</i>'}</strong></div>`).join("")}</div>
      </div>` : ""}
      <div class="drawer-section">
        <div class="drawer-section-head">${icon("note")}<h3>Journal</h3></div>
        <div class="journal">${data.journal.length ? data.journal.map((entry) => `<div class="journal-entry"><span class="journal-date">${esc(entry.timestamp.slice(0, 10))}</span><span>${esc(entry.message)}</span></div>`).join("") : `<span class="field-hint">No journal entries yet</span>`}</div>
        <form class="note-add" id="add-note-form"><textarea class="textarea" id="add-note-input" rows="2" placeholder="Add a journal note…"></textarea><button class="button button-primary" type="submit">${icon("plus")} Add note</button></form>
      </div>
    `}`;
}

// Move-to-base controls for the drawer: a select of the OTHER configured
// bases + a Move button. Hidden when the project isn't in the library or
// there's nowhere else to move to.
function moveControls(record) {
  if (!record?.base) return "";
  const currentKey = (record.base || "").replace(/\/+$/, "");
  const targets = effectiveBases().filter((base) => base.replace(/\/+$/, "") !== currentKey);
  if (!targets.length) return "";
  return `
    <div class="drawer-move">
      <select class="input mono" id="move-base-select" ${state.moving ? "disabled" : ""}>
        ${targets.map((base) => `<option value="${esc(base)}">${esc(baseLabel(base))} — ${esc(base)}</option>`).join("")}
      </select>
      <button class="button button-secondary" data-detail-move ${state.moving ? "disabled" : ""}>${icon("external")} ${state.moving ? "Moving…" : "Move to base"}</button>
    </div>`;
}

async function moveProject() {
  const base = document.querySelector("#move-base-select")?.value;
  if (!base || state.moving) return;
  const detailData = state.detail.data || {};
  const projectPath = state.detail.path;
  const projectId = detailData.metadata?.id || detailData.record?.id || null;
  const projectName = detailData.metadata?.folder || detailData.record?.name || projectPath.split("/").pop();
  state.moving = true;
  render();

  let result;
  try {
    result = await api("/api/project/move", { method: "POST", body: JSON.stringify({ path: projectPath, base }) });
  } catch (error) {
    state.moving = false;
    toast(error.message, true);
    render();
    return;
  }

  // The move runs as a background job (copy → verify → finalize). Watch it to
  // completion so the source is only ever considered gone once verified.
  const status = await showMoveProgress(result.job_id, projectName, baseLabel(base));
  state.moving = false;
  state.searchResults = null;
  await loadState(false);

  if (status === "done") {
    toast(`Moved ${projectName} to ${baseLabel(base)} — verified.`);
    // Re-open the drawer at the project's new location (found by its stable ID).
    const moved = (state.data.projects || []).find((p) => p.id === projectId);
    if (moved) await openProjectDetail(moved.path);
    else { closeDetail(); }
  } else if (status === "cancelled") {
    toast("Move cancelled — original left untouched.");
    render();
  } else {
    render();
  }
}

// Progress overlay for a background move: copy → verify → finalize, with a
// Cancel button. Resolves with the final job status ("done"|"cancelled"|"failed").
function showMoveProgress(jobId, projectName, baseName) {
  return new Promise((resolve) => {
    if (!jobId) { resolve("done"); return; }
    const overlay = document.createElement("div");
    overlay.className = "success-overlay";
    overlay.innerHTML = `
      <div class="success-modal">
        <div class="success-mark move-mark">${icon("external")}</div>
        <h2>Moving ${esc(projectName)}</h2>
        <p>Copying to <strong>${esc(baseName)}</strong>, verifying every file, then removing the original. Your files are never deleted until the copy is verified.</p>
        <div class="job-progress" id="move-progress">
          <div class="job-bar"><div class="job-bar-fill" style="width:0%"></div></div>
          <div class="job-label">Starting…</div>
        </div>
        <div class="success-actions">
          <button class="button button-secondary" data-move-cancel>Cancel</button>
        </div>
      </div>`;
    document.body.append(overlay);
    const node = overlay.querySelector("#move-progress");
    const fill = () => node.querySelector(".job-bar-fill");
    const label = () => node.querySelector(".job-label");
    let cancelling = false;
    overlay.querySelector("[data-move-cancel]").addEventListener("click", async () => {
      if (cancelling) return;
      cancelling = true;
      try { await api(`/api/job/${encodeURIComponent(jobId)}/cancel`, { method: "POST", body: "{}" }); } catch {}
      if (label()) label().textContent = "Cancelling…";
    });

    (async () => {
      let status = "done";
      while (true) {
        let job;
        try {
          const res = await api(`/api/job/${encodeURIComponent(jobId)}`);
          job = res.job;
        } catch { status = "done"; break; } // evicted after finishing → done
        const pct = job.total_bytes
          ? Math.min(100, Math.round((job.copied_bytes / job.total_bytes) * 100))
          : (job.phase === "done" ? 100 : 0);
        if (fill()) fill().style.width = pct + "%";
        node.classList.toggle("job-verifying", job.phase === "verifying");
        if (job.status === "failed") {
          status = "failed";
          node.classList.add("job-failed");
          if (label()) label().textContent = `Move failed: ${job.error || "unknown error"} (original untouched)`;
          toast(`Move failed — original left intact.`, true);
          await new Promise((r) => setTimeout(r, 1600));
          break;
        }
        if (job.status === "cancelled") { status = "cancelled"; break; }
        if (job.status === "done" || job.phase === "done") { status = "done"; break; }
        if (label()) label().textContent = phaseLabel(job);
        await new Promise((r) => setTimeout(r, 400));
      }
      if (status === "done") {
        node.classList.remove("job-verifying");
        node.classList.add("job-done");
        if (fill()) fill().style.width = "100%";
        if (label()) label().textContent = "Moved and verified.";
        await new Promise((r) => setTimeout(r, 500));
      }
      overlay.remove();
      resolve(status);
    })();
  });
}

// Human phase label for a job progress snapshot, shared by create + move.
function phaseLabel(job) {
  const pct = job.total_bytes ? Math.min(100, Math.round((job.copied_bytes / job.total_bytes) * 100)) : 100;
  if (job.phase === "verifying") return `Verifying files… (${job.done_files}/${job.total_files})`;
  if (job.phase === "finalizing") return "Finalizing…";
  if (job.status === "done" || job.phase === "done") return "Done.";
  return `Copying ${job.current_file || "files"}… ${pct}% (${job.done_files}/${job.total_files})`;
}

// Poll one move/copy job to a terminal state, calling `onUpdate(job)` each tick.
// Returns "done" | "cancelled" | "failed". A 404 (evicted after finishing) = done.
async function pollMoveJob(jobId, onUpdate) {
  if (!jobId) return "done";
  while (true) {
    let job;
    try {
      job = (await api(`/api/job/${encodeURIComponent(jobId)}`)).job;
    } catch {
      return "done";
    }
    if (onUpdate) onUpdate(job);
    if (job.status === "failed") return "failed";
    if (job.status === "cancelled") return "cancelled";
    if (job.status === "done" || job.phase === "done") return "done";
    await new Promise((r) => setTimeout(r, 350));
  }
}

// Move every checked project into the chosen base (skipping any already there).
async function moveSelected() {
  if (state.moving) return;
  const base = document.querySelector("#bulk-base-select")?.value || state.bulkBase;
  if (!base) return;
  const baseKey = base.replace(/\/+$/, "");
  const all = state.data.projects || [];
  const targets = all.filter(
    (p) => state.selected.has(p.path) && (p.base || "").replace(/\/+$/, "") !== baseKey
  );
  const skipped = state.selected.size - targets.length;
  if (!targets.length) {
    toast(skipped ? "Every selected project is already in that base." : "Nothing to move.", true);
    return;
  }

  state.moving = true;
  render();
  const result = await runBulkMove(targets, base);
  state.moving = false;
  state.selected.clear();
  state.searchResults = null;
  await loadState(false);
  render();

  const parts = [`${result.done} moved`];
  if (result.failed) parts.push(`${result.failed} failed`);
  if (result.cancelled) parts.push(`${result.cancelled} cancelled`);
  if (skipped) parts.push(`${skipped} already there`);
  toast(parts.join(" · "), result.failed > 0);
}

// Sequential bulk move with one combined overlay (overall bar + per-project
// phase). Moves run one at a time so they never race the base caches, and each
// source is only removed after its copy is verified. Resolves with a tally.
function runBulkMove(projects, base) {
  return new Promise((resolve) => {
    const overlay = document.createElement("div");
    overlay.className = "success-overlay";
    overlay.innerHTML = `
      <div class="success-modal">
        <div class="success-mark move-mark">${icon("external")}</div>
        <h2>Moving ${projects.length} project${projects.length === 1 ? "" : "s"}</h2>
        <p>Copying to <strong>${esc(baseLabel(base))}</strong>, verifying each, then removing the originals. Files are never deleted until the copy is verified.</p>
        <div class="job-progress" id="bulk-progress">
          <div class="job-bar"><div class="job-bar-fill" style="width:0%"></div></div>
          <div class="job-label">Starting…</div>
        </div>
        <div class="success-actions"><button class="button button-secondary" data-bulk-cancel>Cancel</button></div>
      </div>`;
    document.body.append(overlay);
    const node = overlay.querySelector("#bulk-progress");
    const fill = () => node.querySelector(".job-bar-fill");
    const label = () => node.querySelector(".job-label");
    let cancelRequested = false;
    let currentJobId = null;
    overlay.querySelector("[data-bulk-cancel]").addEventListener("click", async () => {
      cancelRequested = true;
      if (currentJobId) {
        try { await api(`/api/job/${encodeURIComponent(currentJobId)}/cancel`, { method: "POST", body: "{}" }); } catch {}
      }
      if (label()) label().textContent = "Cancelling…";
    });

    (async () => {
      let done = 0, failed = 0, cancelled = 0;
      const total = projects.length;
      for (let i = 0; i < total; i++) {
        if (cancelRequested) { cancelled += total - i; break; }
        const project = projects[i];
        if (fill()) fill().style.width = Math.round((i / total) * 100) + "%";
        if (label()) label().textContent = `Moving ${project.name} (${i + 1}/${total})…`;

        let jobId;
        try {
          const res = await api("/api/project/move", { method: "POST", body: JSON.stringify({ path: project.path, base }) });
          jobId = res.job_id;
        } catch {
          failed++;
          continue;
        }
        currentJobId = jobId;
        const status = await pollMoveJob(jobId, (job) => {
          const inner = job.total_bytes ? job.copied_bytes / job.total_bytes : (job.phase === "done" ? 1 : 0);
          if (fill()) fill().style.width = Math.round(((i + inner) / total) * 100) + "%";
          node.classList.toggle("job-verifying", job.phase === "verifying");
          if (label()) label().textContent = `${project.name} — ${phaseLabel(job)} (${i + 1}/${total})`;
        });
        currentJobId = null;
        if (status === "done") done++;
        else if (status === "cancelled") { cancelled++; break; }
        else failed++;
      }
      node.classList.remove("job-verifying");
      node.classList.add(failed ? "job-failed" : "job-done");
      if (fill()) fill().style.width = "100%";
      if (label()) label().textContent = `${done} moved${failed ? `, ${failed} failed` : ""}.`;
      await new Promise((r) => setTimeout(r, 700));
      overlay.remove();
      resolve({ done, failed, cancelled });
    })();
  });
}

// Retry interrupted copies/moves — same recovery the `fastf reconcile` CLI runs.
async function reconcileProvisioning() {
  if (state.reconciling) return;
  state.reconciling = true;
  render();
  try {
    const res = await api("/api/reconcile", { method: "POST", body: "{}" });
    const r = res.report || {};
    const parts = [];
    if (r.resumed) parts.push(`${r.resumed} copy job(s) finished`);
    if (r.completed) parts.push(`${r.completed} move(s) committed`);
    if (r.rolled_back) parts.push(`${r.rolled_back} move(s) rolled back`);
    toast(parts.length ? parts.join(", ") : "Nothing to reconcile.");
    if (r.unrecoverable && r.unrecoverable.length) {
      toast(`${r.unrecoverable.length} item(s) could not be recovered.`, true);
    }
  } catch (error) {
    toast(error.message, true);
  }
  state.reconciling = false;
  await loadState(false);
  render();
}

function bindDrawer() {
  if (!state.detail) return;
  document.querySelectorAll("[data-close-detail]").forEach((element) => element.addEventListener("click", closeDetail));
  document.querySelector("[data-detail-open]")?.addEventListener("click", () => openPath(state.detail.path));
  document.querySelector("[data-detail-copy]")?.addEventListener("click", () => copyPath(state.detail.path));
  document.querySelector("[data-detail-apply]")?.addEventListener("click", () => openApplyModal(state.detail.path));
  document.querySelector("[data-detail-move]")?.addEventListener("click", moveProject);
  document.querySelectorAll("[data-remove-tag]").forEach((button) => button.addEventListener("click", () => mutateTag("remove", button.dataset.removeTag)));
  document.querySelector("#add-tag-form")?.addEventListener("submit", (event) => {
    event.preventDefault();
    const value = document.querySelector("#add-tag-input").value.trim();
    if (value) mutateTag("add", value);
  });
  document.querySelector("#add-note-form")?.addEventListener("submit", (event) => {
    event.preventDefault();
    const value = document.querySelector("#add-note-input").value.trim();
    if (value) addNote(value);
  });
}

async function mutateTag(action, tag) {
  try {
    const result = await api("/api/project/tag", { method: "POST", body: JSON.stringify({ path: state.detail.path, action, tag }) });
    if (state.detail.data?.metadata) state.detail.data.metadata.tags = result.tags;
    await loadState(false);
    state.searchResults = null;
    render();
  } catch (error) {
    toast(error.message, true);
  }
}

async function addNote(message) {
  try {
    const result = await api("/api/project/note", { method: "POST", body: JSON.stringify({ path: state.detail.path, message }) });
    if (state.detail.data) state.detail.data.journal = result.journal;
    render();
    toast("Journal note added.");
  } catch (error) {
    toast(error.message, true);
  }
}

async function copyPath(path) {
  try {
    await navigator.clipboard.writeText(path);
    toast("Path copied to clipboard.");
  } catch {
    toast("Could not copy the path.", true);
  }
}

// ---------------------------------------------------------------------------
// Apply template modal
// ---------------------------------------------------------------------------

function openApplyModal(target = null) {
  const templates = state.data.templates.filter((template) => template.slug !== "(registered)");
  const template = templates[0]?.slug || "";
  const tmpl = templates.find((item) => item.slug === template);
  state.apply = {
    target: target || state.data.config.base_dir,
    template,
    variables: tmpl ? Object.fromEntries(tmpl.variables.map((variable) => [variable.slug, variable.default || ""])) : {},
    preview: null,
    error: "",
    busy: false,
  };
  render();
}

function closeApply() {
  state.apply = null;
  render();
}

function applyModalMarkup() {
  if (!state.apply) return "";
  const apply = state.apply;
  const templates = state.data.templates.filter((template) => template.slug !== "(registered)");
  const tmpl = templates.find((item) => item.slug === apply.template);
  return `
    <div class="overlay" data-close-apply></div>
    <div class="modal modal-wide" role="dialog" aria-modal="true">
      <div class="modal-head"><h2>Apply a template</h2><button class="icon-button" data-close-apply>${icon("close")}</button></div>
      <div class="modal-body scroll">
        <p class="modal-sub">Create any missing folders and files from a template inside an existing folder. Nothing is ever overwritten.</p>
        ${editorField("Target folder", "The folder to fill in", `<input class="input mono" id="apply-target" value="${esc(apply.target)}">`, true, "full")}
        ${editorField("Template", "", `<select class="select" id="apply-template">${templates.map((template) => `<option value="${esc(template.slug)}" ${apply.template === template.slug ? "selected" : ""}>${esc(template.name)}</option>`).join("")}</select>`, true, "full")}
        ${tmpl && tmpl.variables.length ? `<div class="apply-vars">${tmpl.variables.map(applyVariableField).join("")}</div>` : ""}
        ${apply.error ? `<div class="error-box">${esc(apply.error)}</div>` : ""}
        ${apply.preview ? applyPreviewMarkup(apply.preview) : ""}
      </div>
      <div class="modal-foot">
        <button class="button button-secondary" data-apply-preview ${apply.busy ? "disabled" : ""}>${icon("layers")} Preview</button>
        <button class="button button-primary" data-apply-run ${apply.busy || !apply.preview ? "disabled" : ""}>${icon("bolt")} Apply</button>
      </div>
    </div>`;
}

function applyVariableField(variable) {
  const value = state.apply.variables?.[variable.slug] ?? variable.default ?? "";
  const type = variable.type ?? variable.var_type ?? "text";
  const field = type === "select"
    ? `<select class="select" data-apply-var="${esc(variable.slug)}">${variable.options.map((option) => `<option value="${esc(option)}" ${value === option ? "selected" : ""}>${esc(option)}</option>`).join("")}</select>`
    : `<input class="input" data-apply-var="${esc(variable.slug)}" value="${esc(value)}" placeholder="${esc(variable.label)}" autocomplete="off">`;
  return `<div class="field"><div class="field-row"><label>${esc(variable.label)}${variable.required ? '<span class="required">*</span>' : ""}</label></div>${field}</div>`;
}

function applyPreviewMarkup(preview) {
  const creates = preview.actions.filter((item) => item.action === "create").length;
  return `
    <div class="apply-preview">
      <div class="apply-preview-head"><strong>${creates} to create</strong><span>${preview.actions.length - creates} already present</span></div>
      <div class="apply-action-list">${preview.actions.map((item) => `<div class="apply-action ${item.action}"><span class="apply-kind">${item.action === "create" ? "+" : "·"} ${item.kind}</span><code title="${esc(item.path)}">${esc(item.path)}</code></div>`).join("")}</div>
    </div>`;
}

function bindApplyModal() {
  if (!state.apply) return;
  document.querySelectorAll("[data-close-apply]").forEach((element) => element.addEventListener("click", closeApply));
  document.querySelector("#apply-target")?.addEventListener("input", (event) => { state.apply.target = event.target.value; state.apply.preview = null; });
  document.querySelector("#apply-template")?.addEventListener("change", (event) => {
    state.apply.template = event.target.value;
    const tmpl = state.data.templates.find((item) => item.slug === event.target.value);
    state.apply.variables = tmpl ? Object.fromEntries(tmpl.variables.map((variable) => [variable.slug, variable.default || ""])) : {};
    state.apply.preview = null;
    render();
  });
  document.querySelectorAll("[data-apply-var]").forEach((field) => {
    const update = (event) => { state.apply.variables[event.target.dataset.applyVar] = event.target.value; state.apply.preview = null; };
    field.addEventListener("input", update);
    field.addEventListener("change", update);
  });
  document.querySelector("[data-apply-preview]")?.addEventListener("click", runApplyPreview);
  document.querySelector("[data-apply-run]")?.addEventListener("click", runApply);
}

async function runApplyPreview() {
  const apply = state.apply;
  apply.busy = true;
  apply.error = "";
  apply.target = document.querySelector("#apply-target")?.value || apply.target;
  try {
    apply.preview = await api("/api/apply/preview", { method: "POST", body: JSON.stringify({ template: apply.template, variables: apply.variables, target: apply.target }) });
  } catch (error) {
    apply.preview = null;
    apply.error = error.message;
  }
  apply.busy = false;
  render();
}

async function runApply() {
  const apply = state.apply;
  if (!apply.preview) return;
  apply.busy = true;
  apply.error = "";
  try {
    await api("/api/apply", { method: "POST", body: JSON.stringify({ template: apply.template, variables: apply.variables, target: apply.target }) });
    const detailPath = state.detail?.path;
    closeApply();
    toast("Template applied.");
    if (detailPath) openProjectDetail(detailPath);
  } catch (error) {
    apply.error = error.message;
    apply.busy = false;
    render();
  }
}

// ---------------------------------------------------------------------------
// Generate-from-folder modal
// ---------------------------------------------------------------------------

function openFromFolderModal() {
  state.modal = { kind: "from-folder", source: "", slug: "", force: false, bundle: false, busy: false, error: "" };
  render();
}

function closeModal() {
  state.modal = null;
  render();
}

function genericModalMarkup() {
  if (!state.modal) return "";
  if (state.modal.kind === "from-folder") return fromFolderModalMarkup();
  return "";
}

function fromFolderModalMarkup() {
  const modal = state.modal;
  return `
    <div class="overlay" data-close-modal></div>
    <div class="modal" role="dialog" aria-modal="true">
      <div class="modal-head"><h2>Generate from folder</h2><button class="icon-button" data-close-modal>${icon("close")}</button></div>
      <div class="modal-body scroll">
        <p class="modal-sub">Scan an existing folder and turn its structure and text files into a reusable template. Binary and large files are skipped unless you bundle them.</p>
        ${editorField("Source folder", "Absolute path to scan", `<input class="input mono" id="ff-source" value="${esc(modal.source || "")}" placeholder="/home/you/example-project">`, true, "full")}
        ${editorField("New template slug", "Letters, numbers, '-' or '_'", `<input class="input mono" id="ff-slug" value="${esc(modal.slug || "")}" placeholder="my-template">`, true, "full")}
        <label class="inline-check ff-bundle"><input type="checkbox" id="ff-bundle" ${modal.bundle ? "checked" : ""}> Bundle binary &amp; large files (copy them into the template — can be large)</label>
        ${modal.error ? `<div class="error-box">${esc(modal.error)}</div>` : ""}
      </div>
      <div class="modal-foot">
        <label class="inline-check"><input type="checkbox" id="ff-force" ${modal.force ? "checked" : ""}> Overwrite if it exists</label>
        <button class="button button-primary" data-ff-run ${modal.busy ? "disabled" : ""}>${icon("layers")} Generate</button>
      </div>
    </div>`;
}

function bindGenericModal() {
  if (!state.modal) return;
  document.querySelectorAll("[data-close-modal]").forEach((element) => element.addEventListener("click", closeModal));
  if (state.modal.kind === "from-folder") {
    document.querySelector("#ff-source")?.addEventListener("input", (event) => { state.modal.source = event.target.value; });
    document.querySelector("#ff-slug")?.addEventListener("input", (event) => { state.modal.slug = event.target.value; });
    document.querySelector("#ff-force")?.addEventListener("change", (event) => { state.modal.force = event.target.checked; });
    document.querySelector("#ff-bundle")?.addEventListener("change", (event) => { state.modal.bundle = event.target.checked; });
    document.querySelector("[data-ff-run]")?.addEventListener("click", runFromFolder);
  }
}

async function runFromFolder() {
  const modal = state.modal;
  modal.source = document.querySelector("#ff-source")?.value || "";
  modal.slug = document.querySelector("#ff-slug")?.value || "";
  modal.force = document.querySelector("#ff-force")?.checked || false;
  modal.bundle = document.querySelector("#ff-bundle")?.checked || false;
  if (!modal.source.trim() || !modal.slug.trim()) {
    modal.error = "Source folder and slug are both required.";
    render();
    return;
  }
  modal.busy = true;
  modal.error = "";
  render();
  try {
    const result = await api("/api/templates/from-folder", { method: "POST", body: JSON.stringify({ source: modal.source, slug: modal.slug, force: modal.force, bundle_assets: modal.bundle }) });
    await loadState(false);
    const slug = modal.slug;
    state.modal = null;
    state.view = "templates";
    render();
    const r = result.report || {};
    const bundled = r.bundled ? `, ${r.bundled} asset${r.bundled === 1 ? "" : "s"} (${formatBytes(r.bundled_bytes || 0)})` : "";
    toast(`Generated “${slug}” — ${r.folders || 0} folder${r.folders === 1 ? "" : "s"}, ${r.text_files || 0} file${r.text_files === 1 ? "" : "s"}${bundled}.`);
  } catch (error) {
    modal.busy = false;
    modal.error = error.message;
    render();
  }
}

async function loadState(renderAfter = true) {
  state.data = await api("/api/state");
  if (state.view === "template-editor" && !state.templateEditor) {
    const source = initialEditTemplate === "new"
      ? newTemplateDraft()
      : state.data.templates.find((template) => template.slug === initialEditTemplate);
    if (source) {
      state.templateEditor = structuredClone(source);
      state.templateOriginalSlug = initialEditTemplate === "new" ? null : initialEditTemplate;
      state.templateSlugTouched = initialEditTemplate !== "new";
      if (state.templateOriginalSlug) loadTemplateFiles(state.templateOriginalSlug);
    } else {
      state.view = "templates";
    }
  }
  if (!state.selectedTemplate) {
    const requestedTemplate = state.data.templates.some((item) => item.slug === initialTemplate)
      ? initialTemplate
      : "";
    state.selectedTemplate = requestedTemplate || state.data.config.default_template || state.data.templates[0]?.slug;
    const template = state.data.templates.find((item) => item.slug === state.selectedTemplate);
    if (template) initializeVariables(template);
  }
  if (renderAfter) render();
}

async function refresh() {
  try {
    await loadState();
    toast("Workspace refreshed.");
  } catch (error) {
    toast(error.message, true);
  }
}

window.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    if (state.modal) { closeModal(); return; }
    if (state.apply) { closeApply(); return; }
    if (state.detail) { closeDetail(); return; }
  }
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
    event.preventDefault();
    document.querySelector("#global-search")?.focus();
  }
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "n") {
    event.preventDefault();
    state.view = "create";
    render();
  }
});

window.matchMedia?.("(prefers-color-scheme: dark)").addEventListener("change", () => {
  if (state.appearance.theme === "system") applyAppearance();
});

loadState().catch((error) => {
  app.innerHTML = `<div class="boot-screen"><div class="boot-logo"><span></span><span></span></div><div class="boot-wordmark">Fast Folder could not start</div><div style="max-width:420px;color:#9eb0a7;font-size:12px;text-align:center;line-height:1.6">${esc(error.message)}</div></div>`;
});
