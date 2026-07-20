"use client";

import {
  Activity,
  AlertTriangle,
  Bell,
  Bug,
  Check,
  Clipboard,
  ChevronRight,
  CircleDot,
  Clock3,
  Code2,
  Database,
  ExternalLink,
  Filter,
  Inbox,
  LayoutDashboard,
  LoaderCircle,
  LockKeyhole,
  LogOut,
  Menu,
  Paperclip,
  RefreshCw,
  Search,
  Server,
  Settings,
  FolderKanban,
  KeyRound,
  UserRoundCog,
  ScrollText,
  ShieldCheck,
  TerminalSquare,
  Wrench,
  X,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";

const API_URL = process.env.NEXT_PUBLIC_API_URL ?? "/backend-api";
const DEFAULT_PROJECT_ID =
  process.env.NEXT_PUBLIC_PROJECT_ID ?? "00000000-0000-0000-0000-000000000001";

type TrendPoint = { bucket: string; count: number };
type LevelCount = { level: string; count: number };
type EnvelopeItem = { id: string; item_type: string; size_bytes: number; received_at: string };
type Overview = {
  total_events: number;
  events_24h: number;
  unresolved_issues: number;
  resolved_issues: number;
  trends: TrendPoint[];
  levels: LevelCount[];
  recent_items: EnvelopeItem[];
};
type Issue = {
  id: string;
  project_id: string;
  fingerprint: string;
  title: string;
  level: string;
  status: "unresolved" | "in_progress" | "resolved" | "ignored";
  event_count: number;
  first_seen: string;
  last_seen: string;
};
type Event = {
  id: string;
  level: string;
  message: string;
  environment?: string;
  release?: string;
  tags: Record<string, unknown>;
  contexts: Record<string, unknown>;
  exception: Record<string, unknown>;
  received_at: string;
  occurred_at: string;
};
type ProjectInfo = { id:string; external_id:number; name:string; slug:string; created_at:string; last_seen?:string; service_count:number; issue_count:number; event_count:number; latest_release?:string };
type ServiceInfo = { id:string; name:string; environment:string; latest_release?:string; sdk_name?:string; sdk_version?:string; runtime:Record<string,unknown>; event_count:number; log_count:number; issue_count:number; first_seen:string; last_seen:string };
type LogEntry = { id:string; service?:string; environment?:string; occurred_at:string; level:string; body:string; trace_id?:string; attributes:Record<string,unknown> };
type StreamEvent = { id:string; issue_id:string; service?:string; level:string; message:string; environment?:string; release?:string; trace_id?:string; occurred_at:string };
type View = "overview" | "issues" | "services" | "events" | "logs" | "projects" | "users" | "alerts" | "audit" | "setup" | "settings";
type User = { id:string; email:string; display_name:string; role:string };
type AlertRule = { id:string; name:string; level?:string; environment?:string; cooldown_seconds:number; channel:string; target:string; enabled:boolean };
type AuditLog = { id:string; actor_email?:string; action:string; resource_type:string; resource_id?:string; metadata:Record<string,unknown>; occurred_at:string };
type RuntimeConfig = { environment:string; ingest_rate_limit_per_minute:number; retention_days:number; secure_cookies:boolean };
type ProjectListItem = {id:string;external_id:number;name:string;slug:string;retention_days?:number;archived_at?:string;created_at:string};
type ManagedUser = User & {active:boolean;last_login_at?:string;created_at:string};
type ProjectKey = {id:string;name:string;created_at:string;last_used_at?:string;revoked_at?:string};
type Notification = {id:string;rule_name:string;channel:string;status:string;attempts:number;last_error?:string;created_at:string};
type IssueComment = {id:string;author:string;body:string;created_at:string};
type UserSession = {id:string;created_at:string;last_seen_at:string;expires_at:string};

const emptyOverview: Overview = {
  total_events: 0,
  events_24h: 0,
  unresolved_issues: 0,
  resolved_issues: 0,
  trends: [],
  levels: [],
  recent_items: [],
};

async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${API_URL}${path}`, {
    ...init,
    headers: { "content-type": "application/json", ...init?.headers },
  });
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
  if (response.status === 204) return undefined as T;
  return response.json() as Promise<T>;
}

function relativeTime(value: string) {
  const seconds = Math.round((new Date(value).getTime() - Date.now()) / 1000);
  const formatter = new Intl.RelativeTimeFormat("zh-CN", { numeric: "auto" });
  if (Math.abs(seconds) < 60) return formatter.format(seconds, "second");
  const minutes = Math.round(seconds / 60);
  if (Math.abs(minutes) < 60) return formatter.format(minutes, "minute");
  const hours = Math.round(minutes / 60);
  if (Math.abs(hours) < 24) return formatter.format(hours, "hour");
  return formatter.format(Math.round(hours / 24), "day");
}

function compact(value: number) {
  return new Intl.NumberFormat("zh-CN", { notation: "compact" }).format(value);
}

function levelClass(level: string) {
  return ["fatal", "error"].includes(level) ? "critical" : level === "warning" ? "warning" : "info";
}

export default function Home() {
  const [projectId, setProjectId] = useState(DEFAULT_PROJECT_ID);
  const [renderedAt] = useState(() => Date.now());
  const [view, setView] = useState<View>("overview");
  const [overview, setOverview] = useState<Overview>(emptyOverview);
  const [project, setProject] = useState<ProjectInfo | null>(null);
  const [services, setServices] = useState<ServiceInfo[]>([]);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [streamEvents, setStreamEvents] = useState<StreamEvent[]>([]);
  const [issues, setIssues] = useState<Issue[]>([]);
  const [selected, setSelected] = useState<Issue | null>(null);
  const [events, setEvents] = useState<Event[]>([]);
  const [status, setStatus] = useState("unresolved");
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [telemetrySearch, setTelemetrySearch] = useState("");
  const [levelFilter, setLevelFilter] = useState("");
  const [serviceFilter, setServiceFilter] = useState("");
  const [environmentFilter, setEnvironmentFilter] = useState("");
  const [timeRange, setTimeRange] = useState("24h");
  const [fixContext, setFixContext] = useState<unknown | null>(null);
  const [user, setUser] = useState<User | null>(null);
  const [authReady, setAuthReady] = useState(false);
  const [email, setEmail] = useState("admin@example.com");
  const [password, setPassword] = useState("");
  const [authError, setAuthError] = useState("");
  const [authBusy, setAuthBusy] = useState(false);
  const [alertRules, setAlertRules] = useState<AlertRule[]>([]);
  const [auditLogs, setAuditLogs] = useState<AuditLog[]>([]);
  const [runtimeConfig, setRuntimeConfig] = useState<RuntimeConfig | null>(null);
  const [projects, setProjects] = useState<ProjectListItem[]>([]);
  const [managedUsers, setManagedUsers] = useState<ManagedUser[]>([]);
  const [projectKeys, setProjectKeys] = useState<ProjectKey[]>([]);
  const [notifications, setNotifications] = useState<Notification[]>([]);
  const [newProjectName,setNewProjectName]=useState(""); const [newProjectSlug,setNewProjectSlug]=useState("");
  const [newUserEmail,setNewUserEmail]=useState(""); const [newUserName,setNewUserName]=useState("");
  const [inviteUrl,setInviteUrl]=useState(""); const [thresholdCount,setThresholdCount]=useState(""); const [windowSeconds,setWindowSeconds]=useState("300");
  const [createdDsn,setCreatedDsn]=useState("");
  const [comments,setComments]=useState<IssueComment[]>([]); const [commentBody,setCommentBody]=useState("");
  const [userSessions,setUserSessions]=useState<Record<string,UserSession[]>>({});
  const [policyRetention,setPolicyRetention]=useState("30"); const [policyScrub,setPolicyScrub]=useState("password,authorization,cookie,token,secret");
  const [ruleName, setRuleName] = useState("");
  const [ruleTarget, setRuleTarget] = useState("");
  const [ruleChannel, setRuleChannel] = useState("email");
  const sentryDsn = process.env.NEXT_PUBLIC_SENTRY_DSN ?? "http://dev-secret@localhost:8080/1";

  useEffect(() => {
    api<User>("/api/auth/me").then(setUser).catch(() => setUser(null)).finally(() => setAuthReady(true));
  }, []);

  const load = useCallback(async (quiet = false) => {
    if (quiet) setRefreshing(true);
    else setLoading(true);
    try {
      const [nextOverview, nextIssues, nextProject, nextServices] = await Promise.all([
        api<Overview>(`/api/projects/${projectId}/overview`),
        api<Issue[]>(`/api/projects/${projectId}/issues?status=${status}&q=${encodeURIComponent(search)}&limit=100`),
        api<ProjectInfo>(`/api/projects/${projectId}`),
        api<ServiceInfo[]>(`/api/projects/${projectId}/services`),
      ]);
      setOverview(nextOverview);
      setIssues(nextIssues);
      setProject(nextProject);
      setServices(nextServices);
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "无法连接后端");
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, [status, projectId, search]);

  useEffect(() => {
    if (!user) return;
    const initial = window.setTimeout(() => void load(), 0);
    const timer = window.setInterval(() => load(true), 30_000);
    return () => {
      window.clearTimeout(initial);
      window.clearInterval(timer);
    };
  }, [load, user]);

  useEffect(() => {
    if (!selected || !user) return;
    api<Event[]>(`/api/issues/${selected.id}/events?limit=50`)
      .then(setEvents)
      .catch(() => setEvents([]));
    api<IssueComment[]>(`/api/issues/${selected.id}/comments`).then(setComments).catch(()=>setComments([]));
  }, [selected, user]);

  useEffect(() => {
    if (!user || (view !== "logs" && view !== "events")) return;
    const timer = window.setTimeout(() => {
      const params = new URLSearchParams({ limit: "200" });
      if (telemetrySearch) params.set("q", telemetrySearch);
      if (levelFilter) params.set("level", levelFilter);
      if (serviceFilter) params.set("service", serviceFilter);
      if (environmentFilter) params.set("environment", environmentFilter);
      const hours = timeRange === "1h" ? 1 : timeRange === "7d" ? 168 : 24;
      params.set("since", new Date(Date.now() - hours * 3_600_000).toISOString());
      const path = `/api/projects/${projectId}/${view}?${params}`;
      if (view === "logs") api<LogEntry[]>(path).then(setLogs).catch(() => setLogs([]));
      else api<StreamEvent[]>(path).then(setStreamEvents).catch(() => setStreamEvents([]));
    }, 150);
    return () => window.clearTimeout(timer);
  }, [view, telemetrySearch, levelFilter, serviceFilter, environmentFilter, timeRange, user, projectId]);

  useEffect(() => {
    if (!user) return;
    if (view === "alerts") { api<AlertRule[]>(`/api/projects/${projectId}/alert-rules`).then(setAlertRules).catch(()=>setAlertRules([])); api<Notification[]>(`/api/projects/${projectId}/notifications`).then(setNotifications).catch(()=>setNotifications([])); }
    if (view === "projects" || view === "settings") { api<ProjectListItem[]>("/api/projects").then(setProjects); api<ProjectKey[]>(`/api/projects/${projectId}/keys`).then(setProjectKeys); }
    if (view === "users") api<ManagedUser[]>("/api/users").then(setManagedUsers).catch(()=>setManagedUsers([]));
    if (view === "audit") api<AuditLog[]>("/api/audit-logs?limit=100").then(setAuditLogs).catch(()=>setAuditLogs([]));
    if (view === "settings") api<RuntimeConfig>("/api/runtime-config").then(setRuntimeConfig).catch(()=>setRuntimeConfig(null));
  }, [view, user, projectId]);

  async function createAlertRule(event: React.FormEvent) {
    event.preventDefault();
    if (!canManage) return;
    const created=await api<AlertRule>(`/api/projects/${projectId}/alert-rules`,{method:"POST",body:JSON.stringify({name:ruleName,channel:ruleChannel,target:ruleTarget,cooldown_seconds:300,threshold_count:thresholdCount?Number(thresholdCount):null,window_seconds:thresholdCount?Number(windowSeconds):null,notify_recovery:true})});
    setAlertRules(items=>[created,...items]); setRuleName(""); setRuleTarget("");
  }

  async function createProject(e:React.FormEvent){e.preventDefault();const p=await api<ProjectListItem>("/api/projects",{method:"POST",body:JSON.stringify({name:newProjectName,slug:newProjectSlug})});setProjects(v=>[...v,p]);setNewProjectName("");setNewProjectSlug("");setProjectId(p.id);}
  async function rotateKey(){const result=await api<{dsn:string}>(`/api/projects/${projectId}/keys`,{method:"POST",body:JSON.stringify({name:`Console key ${new Date().toLocaleDateString()}`})});setCreatedDsn(result.dsn);setProjectKeys(await api<ProjectKey[]>(`/api/projects/${projectId}/keys`));}
  async function toggleRule(rule:AlertRule){const next=await api<AlertRule>(`/api/alert-rules/${rule.id}`,{method:"PATCH",body:JSON.stringify({enabled:!rule.enabled})});setAlertRules(v=>v.map(r=>r.id===rule.id?next:r));}
  async function resolveCurrentPage(){if(!visibleIssues.length||!canManage)return;await api("/api/issues/batch",{method:"PATCH",body:JSON.stringify({issue_ids:visibleIssues.map(i=>i.id),status:"resolved"})});await load(true);}
  async function addComment(e:React.FormEvent){e.preventDefault();if(!selected)return;const created=await api<IssueComment>(`/api/issues/${selected.id}/comments`,{method:"POST",body:JSON.stringify({body:commentBody})});setComments(v=>[...v,created]);setCommentBody("");}
  async function archiveProject(id:string){if(!confirm("归档项目并立即吊销全部 DSN Key？"))return;await api(`/api/projects/${id}`,{method:"DELETE"});setProjects(v=>v.map(p=>p.id===id?{...p,archived_at:new Date().toISOString()}:p));}
  async function editRule(rule:AlertRule){const name=prompt("规则名称",rule.name);if(!name)return;const target=prompt("通知目标",rule.target);if(!target)return;const next=await api<AlertRule>(`/api/alert-rules/${rule.id}`,{method:"PATCH",body:JSON.stringify({name,target})});setAlertRules(v=>v.map(r=>r.id===rule.id?next:r));}
  async function deleteRule(rule:AlertRule){if(!confirm(`删除规则 ${rule.name}？`))return;await api(`/api/alert-rules/${rule.id}`,{method:"DELETE"});setAlertRules(v=>v.filter(r=>r.id!==rule.id));}
  async function savePolicy(e:React.FormEvent){e.preventDefault();await api(`/api/projects/${projectId}`,{method:"PATCH",body:JSON.stringify({retention_days:Number(policyRetention),scrub_fields:policyScrub.split(",").map(v=>v.trim()).filter(Boolean)})});}
  async function loadUserSessions(userId:string){const sessions=await api<UserSession[]>(`/api/users/${userId}/sessions`);setUserSessions(v=>({...v,[userId]:sessions}));}
  async function inviteUser(e:React.FormEvent){e.preventDefault();const result=await api<{invite_url:string}>("/api/invitations",{method:"POST",body:JSON.stringify({email:newUserEmail,display_name:newUserName,role:"member"})});setInviteUrl(result.invite_url);}
  async function requestDeletion(){if(!project)return;const confirmation=prompt(`输入 ${project.slug} 申请永久删除`);if(!confirmation)return;await api(`/api/projects/${projectId}/deletion-request`,{method:"POST",body:JSON.stringify({confirmation})});alert("删除申请已创建，24 小时内可取消，之后才能永久清除。")}
  async function changeIssueFingerprint(issue:Issue){const fingerprint=prompt("新的 fingerprint",issue.fingerprint);if(!fingerprint)return;await api(`/api/issues/${issue.id}/fingerprint`,{method:"PATCH",body:JSON.stringify({fingerprint})});await load(true);}
  async function mergeIssue(issue:Issue){const target_issue_id=prompt("合并到目标 Issue ID");if(!target_issue_id)return;await api("/api/issues/merge",{method:"POST",body:JSON.stringify({target_issue_id,source_issue_ids:[issue.id]})});setSelected(null);await load(true);}
  async function splitIssue(issue:Issue){const event_id=prompt("要拆分的 Event ID",events[0]?.id);if(!event_id)return;const fingerprint=prompt("新 Issue fingerprint");if(!fingerprint)return;await api(`/api/issues/${issue.id}/split`,{method:"POST",body:JSON.stringify({event_ids:[event_id],fingerprint})});setSelected(null);await load(true);}

  async function signIn(event: React.FormEvent) {
    event.preventDefault(); setAuthBusy(true); setAuthError("");
    try { setUser(await api<User>("/api/auth/login", { method:"POST", body:JSON.stringify({email,password}) })); setPassword(""); }
    catch { setAuthError("邮箱或密码不正确"); }
    finally { setAuthBusy(false); }
  }

  async function signOut() { await api("/api/auth/logout", {method:"POST",body:"{}"}).catch(()=>undefined); setUser(null); setOverview(emptyOverview); }

  const visibleIssues = (() => {
    const needle = search.trim().toLowerCase();
    if (!needle) return issues;
    return issues.filter((issue) =>
      [issue.title, issue.level, issue.fingerprint].some((value) => value.toLowerCase().includes(needle)),
    );
  })();

  async function resolveIssue(issue: Issue) {
    const nextStatus = issue.status === "resolved" ? "unresolved" : "resolved";
    await api(`/api/issues/${issue.id}`, {
      method: "PATCH",
      body: JSON.stringify({ status: nextStatus }),
    });
    setSelected({ ...issue, status: nextStatus });
    await load(true);
  }

  async function ignoreIssue(issue: Issue) {
    await api(`/api/issues/${issue.id}`, { method:"PATCH", body:JSON.stringify({ status:"ignored" }) });
    setSelected(null);
    await load(true);
  }

  async function startFix(issue: Issue) {
    const context = await api<unknown>(`/api/issues/${issue.id}`, { method:"POST", body:"{}" });
    setFixContext(context);
    setSelected({ ...issue, status:"in_progress" });
    await navigator.clipboard.writeText(JSON.stringify(context, null, 2)).catch(() => undefined);
    await load(true);
  }

  const maxTrend = Math.max(1, ...overview.trends.map((point) => point.count));
  const totalLevels = Math.max(1, overview.levels.reduce((sum, item) => sum + item.count, 0));
  const canManage = user?.role === "owner" || user?.role === "admin";

  if (!authReady) return <div className="auth-loading"><LoaderCircle className="spin"/><span>正在验证会话…</span></div>;
  if (!user) return <main className="login-page"><section className="login-card"><div className="login-brand"><span className="brand-mark"><ShieldCheck size={21}/></span><span>Open Sentry</span></div><div className="login-copy"><span className="login-icon"><LockKeyhole size={22}/></span><p className="section-kicker">SECURE CONSOLE</p><h1>登录监控工作区</h1><p>查看生产事件、服务健康状态和修复队列。</p></div><form onSubmit={signIn}><label>邮箱<input type="email" value={email} onChange={e=>setEmail(e.target.value)} autoComplete="username" required/></label><label>密码<input type="password" value={password} onChange={e=>setPassword(e.target.value)} autoComplete="current-password" minLength={8} required placeholder="至少 8 位"/></label>{authError&&<div className="login-error" role="alert">{authError}</div>}<button disabled={authBusy}>{authBusy?<LoaderCircle className="spin" size={17}/>:<LockKeyhole size={17}/>}登录</button></form><small>凭据由部署环境的管理员配置。</small></section></main>;

  return (
    <div className="app-shell">
      <aside className={`sidebar ${menuOpen ? "sidebar-open" : ""}`}>
        <div className="brand"><span className="brand-mark"><ShieldCheck size={19} /></span><span>Open Sentry</span></div>
        <button className="mobile-close" onClick={() => setMenuOpen(false)} aria-label="关闭菜单"><X /></button>
        <nav className="main-nav">
          <p className="nav-label">工作区</p>
          <button className={`nav-item ${view === "overview" ? "active" : ""}`} onClick={() => setView("overview")}><LayoutDashboard size={17} />概览</button>
          <button className={`nav-item ${view === "issues" ? "active" : ""}`} onClick={() => setView("issues")}><Bug size={17} />Issues <span className="nav-count">{overview.unresolved_issues}</span></button>
          <button className={`nav-item ${view === "services" ? "active" : ""}`} onClick={() => setView("services")}><Server size={17} />Services <span className="nav-count">{services.length}</span></button>
          <button className={`nav-item ${view === "events" ? "active" : ""}`} onClick={() => setView("events")}><Activity size={17} />Events</button>
          <button className={`nav-item ${view === "logs" ? "active" : ""}`} onClick={() => setView("logs")}><TerminalSquare size={17} />Logs</button>
          <p className="nav-label nav-gap">管理</p>
          <button className={`nav-item ${view === "projects" ? "active" : ""}`} onClick={()=>setView("projects")}><FolderKanban size={17}/>项目管理</button>
          {canManage&&<button className={`nav-item ${view === "users" ? "active" : ""}`} onClick={()=>setView("users")}><UserRoundCog size={17}/>成员权限</button>}
          <button className={`nav-item ${view === "alerts" ? "active" : ""}`} onClick={()=>setView("alerts")}><Bell size={17} />告警规则</button>
          <button className={`nav-item ${view === "audit" ? "active" : ""}`} onClick={()=>setView("audit")}><ScrollText size={17} />审计日志</button>
          <button className={`nav-item ${view === "setup" ? "active" : ""}`} onClick={()=>setView("setup")}><TerminalSquare size={17} />SDK 接入</button>
          <button className={`nav-item ${view === "settings" ? "active" : ""}`} onClick={()=>setView("settings")}><Settings size={17} />项目设置</button>
        </nav>
        <div className="project-switcher">
          <span className="project-avatar">D</span>
          <span><strong>{project?.name ?? "Demo Project"}</strong><small>{project?.slug ?? "production"}</small></span>
          <ChevronRight size={16} />
        </div>
      </aside>

      {menuOpen && <button className="scrim" onClick={() => setMenuOpen(false)} aria-label="关闭菜单" />}

      <main className="main-content">
        <header className="topbar">
          <button className="menu-button" onClick={() => setMenuOpen(true)} aria-label="打开菜单"><Menu /></button>
          <div className="page-title"><span className="eyebrow">{project?.name ?? "Demo Project"} /</span><h1>{{ overview:"运行概览", issues:"Issues", services:"Services", events:"事件流", logs:"Logs",projects:"项目管理",users:"成员权限",alerts:"告警规则",audit:"审计日志",setup:"SDK 接入",settings:"项目设置" }[view]}</h1></div>
          <div className="topbar-actions">
            <span className={`health-pill ${error ? "offline" : ""}`}><span />{error ? "连接中断" : "系统正常"}</span>
            <button className="icon-button" onClick={() => load(true)} aria-label="刷新数据"><RefreshCw className={refreshing ? "spin" : ""} size={18} /></button>
            <button className="user-menu" onClick={signOut} title={`退出 ${user.email}`}><span className="avatar">{user.display_name.slice(0,2).toUpperCase()}</span><span className="user-copy"><strong>{user.display_name}</strong><small>{user.role}</small></span><LogOut size={15}/></button>
          </div>
        </header>

        <div className="content-wrap">
          {error && (
            <div className="connection-banner"><AlertTriangle size={18} /><div><strong>无法读取实时数据</strong><span>确认 Rust 服务运行在 {API_URL}。错误：{error}</span></div><button onClick={() => load()}>重试</button></div>
          )}

          {view === "overview" && <>
          <section className="project-hero panel">
            <div><span className="project-avatar large">{project?.name?.[0] ?? "D"}</span><div><p className="section-kicker">PROJECT #{project?.external_id ?? 1}</p><h2>{project?.name ?? "Demo Project"}</h2><span className="project-slug">{project?.slug} · 创建于 {project ? new Date(project.created_at).toLocaleDateString("zh-CN") : "—"}</span></div></div>
            <div className="project-facts"><span><small>Services</small><strong>{project?.service_count ?? 0}</strong></span><span><small>Issues</small><strong>{project?.issue_count ?? 0}</strong></span><span><small>Latest release</small><strong>{project?.latest_release ?? "—"}</strong></span><span><small>Last event</small><strong>{project?.last_seen ? relativeTime(project.last_seen) : "—"}</strong></span></div>
          </section>
          <section className="summary-row">
            <article className="metric-card accent-card"><div className="metric-head"><span>24 小时事件</span><Activity size={18} /></div><strong>{loading ? "—" : compact(overview.events_24h)}</strong><small>累计 {compact(overview.total_events)} 个事件</small></article>
            <article className="metric-card"><div className="metric-head"><span>未解决 Issues</span><AlertTriangle size={18} /></div><strong>{loading ? "—" : compact(overview.unresolved_issues)}</strong><small className="danger-copy">需要团队关注</small></article>
            <article className="metric-card"><div className="metric-head"><span>已解决</span><Check size={18} /></div><strong>{loading ? "—" : compact(overview.resolved_issues)}</strong><small>所有历史处理记录</small></article>
            <article className="metric-card"><div className="metric-head"><span>采集状态</span><Database size={18} /></div><strong className="status-value"><CircleDot size={16} />实时</strong><small>每 30 秒自动刷新</small></article>
          </section>

          <section className="insight-grid">
            <article className="panel trend-panel">
              <div className="panel-header"><div><p className="section-kicker">事件吞吐</p><h2>最近 24 小时</h2></div><span className="panel-meta">按小时</span></div>
              <div className="chart-wrap">
                <div className="chart-y"><span>{maxTrend}</span><span>{Math.round(maxTrend / 2)}</span><span>0</span></div>
                <div className="bar-chart" aria-label="24 小时事件趋势">
                  {overview.trends.map((point, index) => <div className="bar-slot" key={point.bucket} title={`${new Date(point.bucket).toLocaleString("zh-CN")} · ${point.count} 个事件`}><span className={point.count ? "bar active-bar" : "bar"} style={{ height: `${Math.max(point.count ? 8 : 2, (point.count / maxTrend) * 100)}%` }} />{index % 6 === 0 && <small>{new Date(point.bucket).getHours()}:00</small>}</div>)}
                  {!overview.trends.length && <div className="chart-empty">等待第一批事件进入</div>}
                </div>
              </div>
            </article>
            <article className="panel level-panel">
              <div className="panel-header"><div><p className="section-kicker">严重程度</p><h2>事件分布</h2></div><Filter size={17} /></div>
              <div className="level-stack">{overview.levels.map((item) => <div className="level-row" key={item.level}><span className={`level-dot ${levelClass(item.level)}`} /><span className="level-name">{item.level}</span><div className="level-track"><span className={levelClass(item.level)} style={{ width: `${(item.count / totalLevels) * 100}%` }} /></div><strong>{item.count}</strong></div>)}{!overview.levels.length && <div className="small-empty">暂无级别数据</div>}</div>
              <div className="level-total"><span>已索引事件</span><strong>{overview.total_events}</strong></div>
            </article>
          </section>

          <section className="panel issues-panel">
            <div className="issues-toolbar"><div><p className="section-kicker">优先处理</p><h2>Issue 流</h2></div><div className="toolbar-controls"><label className="search-box"><Search size={16} /><input value={search} onChange={(event) => setSearch(event.target.value)} placeholder="搜索标题、级别或指纹" /></label><select value={status} onChange={(event) => setStatus(event.target.value)} aria-label="Issue 状态"><option value="unresolved">未解决</option><option value="in_progress">修复中</option><option value="resolved">已解决</option><option value="ignored">已忽略</option></select></div></div>
            <div className="issue-table">
              <div className="issue-row issue-head"><span>问题</span><span>级别</span><span>事件数</span><span>首次出现</span><span>最后出现</span><span /></div>
              {visibleIssues.map((issue) => <button className="issue-row" key={issue.id} onClick={() => setSelected(issue)}><span className="issue-main"><span className={`issue-icon ${levelClass(issue.level)}`}><Bug size={17} /></span><span><strong>{issue.title}</strong><small>{issue.fingerprint.slice(0, 12)} · {issue.status}</small></span></span><span><span className={`level-badge ${levelClass(issue.level)}`}>{issue.level}</span></span><strong className="event-count">{issue.event_count}</strong><span>{relativeTime(issue.first_seen)}</span><span>{relativeTime(issue.last_seen)}</span><ChevronRight size={17} /></button>)}
              {!loading && !visibleIssues.length && <div className="empty-state"><Inbox size={28} /><strong>这个视图很安静</strong><span>当前筛选条件下没有 Issue。</span></div>}
              {loading && <div className="loading-state"><LoaderCircle className="spin" />正在读取 Issue…</div>}
            </div>
          </section>

          <section className="panel raw-panel">
            <div className="panel-header"><div><p className="section-kicker">协议观测</p><h2>最近 Envelope Items</h2></div><Code2 size={18} /></div>
            <div className="raw-items">{overview.recent_items.map((item) => <div className="raw-item" key={item.id}><span className="raw-icon">{item.item_type === "attachment" ? <Paperclip size={17} /> : <Code2 size={17} />}</span><span><strong>{item.item_type}</strong><small>{item.id.slice(0, 8)}</small></span><span className="raw-size">{(item.size_bytes / 1024).toFixed(1)} KB</span>{item.item_type==="attachment"&&<a href={`${API_URL}/api/envelope-items/${item.id}/download`}>下载</a>}<time>{relativeTime(item.received_at)}</time></div>)}{!overview.recent_items.length && <div className="small-empty">Attachment、Log、Session 等原始 Item 将显示在这里。</div>}</div>
          </section>
          </>}

          {view === "issues" && <section className="panel issues-panel standalone-panel">
            <div className="issues-toolbar"><div><p className="section-kicker">工作队列</p><h2>所有 Issues</h2></div><div className="toolbar-controls"><label className="search-box"><Search size={16} /><input value={search} onChange={(event) => setSearch(event.target.value)} placeholder="服务端搜索标题或指纹" /></label><select value={status} onChange={(event) => setStatus(event.target.value)}><option value="unresolved">未解决</option><option value="in_progress">修复中</option><option value="resolved">已解决</option><option value="ignored">已忽略</option></select>{canManage&&<button className="clear-filter" onClick={resolveCurrentPage}><Check size={14}/>解决当前页</button>}</div></div>
            <div className="issue-table"><div className="issue-row issue-head"><span>问题</span><span>级别</span><span>事件数</span><span>首次出现</span><span>最后出现</span><span /></div>{visibleIssues.map((issue) => <button className="issue-row" key={issue.id} onClick={() => setSelected(issue)}><span className="issue-main"><span className={`issue-icon ${levelClass(issue.level)}`}><Bug size={17}/></span><span><strong>{issue.title}</strong><small>{issue.fingerprint.slice(0,12)} · {issue.status}</small></span></span><span><span className={`level-badge ${levelClass(issue.level)}`}>{issue.level}</span></span><strong>{issue.event_count}</strong><span>{relativeTime(issue.first_seen)}</span><span>{relativeTime(issue.last_seen)}</span><ChevronRight size={17}/></button>)}</div>
          </section>}

          {view === "services" && <div className="view-stack">
            <section className="view-heading"><div><p className="section-kicker">服务目录</p><h2>{services.length} 个活跃服务</h2><p>根据 SDK Event 与 Log 的 service 信息自动聚合。</p></div></section>
            <div className="service-grid">{services.map((service) => <article className="service-card panel" key={service.id}><div className="service-card-head"><span className="service-mark"><Server size={18}/></span><span className={`service-health ${renderedAt-new Date(service.last_seen).getTime()<86400000 ? "healthy":"stale"}`}>{renderedAt-new Date(service.last_seen).getTime()<86400000 ? "Healthy":"Stale"}</span></div><h3>{service.name}</h3><p>{service.environment} · {service.latest_release ?? "no release"}</p><div className="service-metrics"><span><strong>{service.event_count}</strong><small>Events</small></span><span><strong>{service.log_count}</strong><small>Logs</small></span><span><strong>{service.issue_count}</strong><small>Issues</small></span></div><div className="service-meta"><span>{service.sdk_name ?? "unknown SDK"} {service.sdk_version}</span><span>活跃于 {relativeTime(service.last_seen)}</span></div><button onClick={() => { setServiceFilter(service.name); setView("logs"); }}>查看服务日志 <ChevronRight size={15}/></button></article>)}</div>
          </div>}

          {(view === "logs" || view === "events") && <div className="view-stack">
            <section className="telemetry-toolbar panel"><div><p className="section-kicker">{view === "logs" ? "结构化 SDK LOGS":"逐条事件"}</p><h2>{view === "logs" ? `${logs.length} 条日志`:`${streamEvents.length} 个事件`}</h2></div><div className="telemetry-filters"><label className="search-box wide"><Search size={16}/><input value={telemetrySearch} onChange={(event)=>setTelemetrySearch(event.target.value)} placeholder={`搜索${view === "logs" ? "日志正文":"事件消息"}`}/></label><select value={timeRange} onChange={(e)=>setTimeRange(e.target.value)}><option value="1h">最近 1h</option><option value="24h">最近 24h</option><option value="7d">最近 7d</option></select><select value={levelFilter} onChange={(e)=>setLevelFilter(e.target.value)}><option value="">全部级别</option><option value="fatal">fatal</option><option value="error">error</option><option value="warn">warn</option><option value="warning">warning</option><option value="info">info</option></select><select value={serviceFilter} onChange={(e)=>setServiceFilter(e.target.value)}><option value="">全部服务</option>{services.map(s=><option value={s.name} key={s.id}>{s.name}</option>)}</select><select value={environmentFilter} onChange={(e)=>setEnvironmentFilter(e.target.value)}><option value="">全部环境</option>{[...new Set(services.map(s=>s.environment))].map(env=><option value={env} key={env}>{env}</option>)}</select>{(telemetrySearch||levelFilter||serviceFilter||environmentFilter)&&<button className="clear-filter" onClick={()=>{setTelemetrySearch("");setLevelFilter("");setServiceFilter("");setEnvironmentFilter("");}}><X size={14}/>清除</button>}</div></section>
            {view === "logs" ? <section className="panel log-console"><div className="log-head"><span>时间</span><span>Level</span><span>Service</span><span>日志正文</span><span>Trace ID</span></div>{logs.map(log=><details className="log-line" key={log.id}><summary><time>{new Date(log.occurred_at).toLocaleTimeString("zh-CN",{hour12:false})}</time><span className={`log-level ${levelClass(log.level)}`}>{log.level}</span><button onClick={(e)=>{e.preventDefault();setServiceFilter(log.service??"");}}>{log.service??"unknown-service"}</button><strong>{log.body}</strong><code>{log.trace_id?.slice(0,12)??"—"}</code></summary><pre>{JSON.stringify(log.attributes,null,2)}</pre></details>)}{!logs.length&&<div className="empty-state"><TerminalSquare/><strong>没有匹配的日志</strong><span>调整筛选条件或运行 sentry-smoke。</span></div>}</section>
            : <section className="panel event-stream"><div className="log-head event-head"><span>时间</span><span>Level</span><span>Service</span><span>事件</span><span>Release / Trace</span></div>{streamEvents.map(event=><button className="event-line" key={event.id} onClick={()=>{const issue=issues.find(i=>i.id===event.issue_id);if(issue)setSelected(issue);}}><time>{new Date(event.occurred_at).toLocaleTimeString("zh-CN",{hour12:false})}</time><span className={`log-level ${levelClass(event.level)}`}>{event.level}</span><span>{event.service??"unknown-service"}</span><strong>{event.message}</strong><code>{event.release??event.trace_id?.slice(0,12)??"—"}</code></button>)}</section>}
          </div>}

          {view === "projects"&&<div className="management-grid"><section className="panel management-panel"><p className="section-kicker">PROJECTS</p><h2>项目与数据边界</h2><div className="rule-list">{projects.filter(p=>!p.archived_at).map(p=><article key={p.id}><span className="project-avatar">{p.name[0]}</span><span><strong>{p.name}</strong><small>{p.slug} · #{p.external_id} · 保留 {p.retention_days??runtimeConfig?.retention_days??30} 天</small></span><button onClick={()=>setProjectId(p.id)}>打开</button>{canManage&&p.id!==projectId&&<button onClick={()=>archiveProject(p.id)}>归档</button>}</article>)}</div></section>{canManage&&<section className="panel management-panel"><p className="section-kicker">CREATE PROJECT</p><h2>创建项目</h2><form className="settings-form" onSubmit={createProject}><label>项目名称<input required value={newProjectName} onChange={e=>setNewProjectName(e.target.value)}/></label><label>Slug<input required pattern="[a-z0-9-]+" value={newProjectSlug} onChange={e=>setNewProjectSlug(e.target.value.toLowerCase())}/></label><button>创建并打开</button></form></section>}</div>}

          {view === "users"&&canManage&&<div className="management-grid"><section className="panel management-panel"><p className="section-kicker">MEMBERS</p><h2>成员与角色</h2><div className="rule-list">{managedUsers.map(member=><div key={member.id}><article><span className={`status-dot ${member.active?"on":""}`}/><span><strong>{member.display_name}</strong><small>{member.email} · {member.last_login_at?`登录于 ${relativeTime(member.last_login_at)}`:"尚未登录"}</small></span><select value={member.role} disabled={member.id===user.id} onChange={async e=>{const next=await api<ManagedUser>(`/api/users/${member.id}`,{method:"PATCH",body:JSON.stringify({role:e.target.value})});setManagedUsers(v=>v.map(x=>x.id===member.id?next:x));}}><option value="owner">owner</option><option value="admin">admin</option><option value="member">member</option></select><button onClick={() => loadUserSessions(member.id)}>Sessions</button>{member.id!==user.id&&member.active&&<button onClick={async()=>{await api(`/api/users/${member.id}`,{method:"DELETE"});setManagedUsers(v=>v.map(x=>x.id===member.id?{...x,active:false}:x));}}>停用</button>}</article>{userSessions[member.id]?.map(session=><article key={session.id}><Clock3 size={15}/><span><strong>活跃 Session</strong><small>最后活动 {relativeTime(session.last_seen_at)} · 到期 {new Date(session.expires_at).toLocaleDateString()}</small></span><button onClick={async()=>{await api(`/api/sessions/${session.id}`,{method:"DELETE"});setUserSessions(v=>({...v,[member.id]:v[member.id].filter(s=>s.id!==session.id)}));}}>撤销</button></article>)}</div>)}</div></section><section className="panel management-panel"><p className="section-kicker">ADD MEMBER</p><h2>创建成员</h2><form className="settings-form" onSubmit={inviteUser}><label>姓名<input required value={newUserName} onChange={e=>setNewUserName(e.target.value)}/></label><label>邮箱<input type="email" required value={newUserEmail} onChange={e=>setNewUserEmail(e.target.value)}/></label><button>生成邀请链接</button>{inviteUrl&&<div className="dsn-box"><code>{inviteUrl}</code><button type="button" onClick={()=>navigator.clipboard.writeText(inviteUrl)}>复制</button></div>}</form></section></div>}

          {view === "alerts" && <div className="management-grid"><section className="panel management-panel"><div className="view-heading"><div><p className="section-kicker">通知策略</p><h2>告警规则</h2><p>匹配事件后通过可靠 Outbox 发送通知。</p></div></div><div className="rule-list">{alertRules.map(rule=><article key={rule.id}><span className={`status-dot ${rule.enabled?"on":""}`}/><span><strong>{rule.name}</strong><small>{rule.channel} · {rule.target} · 冷却 {rule.cooldown_seconds}s</small></span><button disabled={!canManage} onClick={()=>toggleRule(rule)}>{rule.enabled?"停用":"启用"}</button><button disabled={!canManage} onClick={()=>editRule(rule)}>编辑</button><button disabled={!canManage} onClick={()=>api(`/api/alert-rules/${rule.id}/test`,{method:"POST",body:"{}"})}>测试</button><button onClick={async()=>{const result=await api<{ok:boolean,error?:string}>(`/api/alert-rules/${rule.id}/check`);alert(result.ok?"渠道配置正常":result.error)}}>检查渠道</button><button disabled={!canManage} onClick={()=>deleteRule(rule)}>删除</button></article>)}{!alertRules.length&&<div className="small-empty">尚未配置告警规则</div>}</div><h3>最近投递</h3><div className="rule-list">{notifications.slice(0,8).map(n=><article key={n.id}><span className={`status-dot ${n.status==="sent"?"on":""}`}/><span><strong>{n.rule_name}</strong><small>{n.status} · 尝试 {n.attempts} 次 {n.last_error&&`· ${n.last_error}`}</small></span>{n.status==="failed"&&<button onClick={()=>api(`/api/notifications/${n.id}/retry`,{method:"POST",body:"{}"})}>重试</button>}</article>)}</div></section><section className="panel management-panel"><p className="section-kicker">CREATE RULE</p><h2>创建告警</h2><form className="settings-form" onSubmit={createAlertRule}><label>规则名称<input value={ruleName} onChange={e=>setRuleName(e.target.value)} required placeholder="生产错误告警"/></label><label>通知渠道<select value={ruleChannel} onChange={e=>setRuleChannel(e.target.value)}><option value="email">Email</option><option value="telegram">Telegram</option><option value="voice_call">电话叫醒（Twilio）</option></select></label><label>{ruleChannel==="email"?"收件邮箱":ruleChannel==="voice_call"?"手机号（E.164）":"Chat ID"}<input value={ruleTarget} onChange={e=>setRuleTarget(e.target.value)} required placeholder={ruleChannel==="email"?"oncall@example.com":ruleChannel==="voice_call"?"+60123456789":"123456789"}/></label><label>窗口阈值<input type="number" min="1" value={thresholdCount} onChange={e=>setThresholdCount(e.target.value)} placeholder="留空则每次触发"/></label>{thresholdCount&&<label>窗口秒数<input type="number" min="60" value={windowSeconds} onChange={e=>setWindowSeconds(e.target.value)}/></label>}<button disabled={!canManage}>创建规则</button></form></section></div>}

          {view === "audit" && <section className="panel management-panel"><div className="view-heading"><div><p className="section-kicker">SECURITY TRAIL</p><h2>组织审计日志</h2><p>关键身份和配置变更的不可变记录。</p></div></div><div className="audit-table"><div className="audit-row audit-head"><span>时间</span><span>操作者</span><span>动作</span><span>资源</span></div>{auditLogs.map(item=><div className="audit-row" key={item.id}><time>{new Date(item.occurred_at).toLocaleString("zh-CN")}</time><span>{item.actor_email??"system"}</span><strong>{item.action}</strong><code>{item.resource_type}{item.resource_id?` · ${item.resource_id.slice(0,8)}`:""}</code></div>)}</div></section>}

          {view === "setup" && <div className="management-grid"><section className="panel management-panel"><p className="section-kicker">RUST SDK</p><h2>连接应用</h2><p className="management-copy">使用项目 DSN 初始化最新 Sentry Rust SDK。该凭证只用于事件接入，不能访问管理 API。</p><div className="dsn-box"><code>{sentryDsn}</code><button onClick={()=>navigator.clipboard.writeText(sentryDsn)}><Clipboard size={15}/>复制</button></div><pre className="code-block">{`let _guard = sentry::init((\n  "${sentryDsn}",\n  sentry::ClientOptions::default(),\n));`}</pre></section><section className="panel management-panel"><p className="section-kicker">ACCEPTANCE</p><h2>发送覆盖测试</h2><p className="management-copy">仓库内置 message、error、event、attachment、transaction、logs 和 panic 覆盖客户端。</p><pre className="code-block">cargo run --bin sentry-smoke -- all</pre></section></div>}

          {view === "settings" && <div className="view-stack"><section className="panel management-panel"><div className="view-heading"><div><p className="section-kicker">RUNTIME POLICY</p><h2>项目与运行策略</h2><p>项目密钥只在创建时显示一次，服务端仅保存 SHA-256 摘要。</p></div></div><div className="settings-cards"><article><small>运行环境</small><strong>{runtimeConfig?.environment??"—"}</strong></article><article><small>接入配额</small><strong>{runtimeConfig?.ingest_rate_limit_per_minute??"—"} / min</strong></article><article><small>默认数据保留</small><strong>{runtimeConfig?.retention_days??"—"} 天</strong></article><article><small>安全 Cookie</small><strong>{runtimeConfig?.secure_cookies?"已启用":"本地模式"}</strong></article><article><small>Project ID</small><code>{projectId}</code></article><article><small>Numeric ID</small><strong>#{project?.external_id??1}</strong></article></div>{canManage&&<form className="settings-form" onSubmit={savePolicy}><label>本项目保留天数<input type="number" min="1" value={policyRetention} onChange={e=>setPolicyRetention(e.target.value)}/></label><label>敏感字段（逗号分隔）<input value={policyScrub} onChange={e=>setPolicyScrub(e.target.value)}/></label><button>保存策略</button><button type="button" onClick={requestDeletion}>申请永久删除</button></form>}</section><section className="panel management-panel"><div className="view-heading"><div><p className="section-kicker">DSN KEYS</p><h2>接入密钥</h2></div>{canManage&&<button onClick={rotateKey}><KeyRound size={16}/>生成新 Key</button>}</div>{createdDsn&&<div className="dsn-box"><code>{createdDsn}</code><button onClick={()=>navigator.clipboard.writeText(createdDsn)}><Clipboard size={15}/>复制</button></div>}<div className="rule-list">{projectKeys.map(key=><article key={key.id}><span className={`status-dot ${!key.revoked_at?"on":""}`}/><span><strong>{key.name}</strong><small>{key.revoked_at?"已吊销":key.last_used_at?`最后使用 ${relativeTime(key.last_used_at)}`:"从未使用"}</small></span>{canManage&&!key.revoked_at&&<button onClick={async()=>{await api(`/api/projects/${projectId}/keys/${key.id}`,{method:"DELETE"});setProjectKeys(v=>v.map(x=>x.id===key.id?{...x,revoked_at:new Date().toISOString()}:x));}}>吊销</button>}</article>)}</div></section></div>}
        </div>
      </main>

      {selected && <aside className="detail-drawer"><div className="drawer-head"><div><span className={`level-badge ${levelClass(selected.level)}`}>{selected.level}</span><h2>{selected.title}</h2><p>{selected.id}</p></div><button className="icon-button" onClick={() => {setSelected(null);setFixContext(null);}} aria-label="关闭详情"><X size={19} /></button></div><div className="drawer-actions"><button className="fix-action" disabled={!canManage} onClick={() => startFix(selected)}><Wrench size={16}/>Fix</button><button className="primary-action" disabled={!canManage} onClick={() => resolveIssue(selected)}>{selected.status === "resolved" ? <RefreshCw size={16} /> : <Check size={16} />}{selected.status === "resolved" ? "重新打开" : "解决"}</button><button className="secondary-action" disabled={!canManage} onClick={() => ignoreIssue(selected)}>忽略</button><button className="secondary-action" disabled={!canManage} onClick={()=>mergeIssue(selected)}>合并</button><button className="secondary-action" disabled={!canManage} onClick={()=>splitIssue(selected)}>拆分</button><button className="secondary-action" onClick={() => navigator.clipboard.writeText(location.href)}><ExternalLink size={16} />复制链接</button></div>{fixContext && <div className="fix-banner"><Check size={17}/><span><strong>已进入修复中</strong>修复上下文已复制，可直接交给 Codex。</span></div>}<div className="drawer-stats"><div><span>事件</span><strong>{selected.event_count}</strong></div><div><span>状态</span><strong>{selected.status}</strong></div><div><span>最后出现</span><strong>{relativeTime(selected.last_seen)}</strong></div></div><div className="drawer-section"><h3><Clock3 size={16} />事件时间线</h3><div className="timeline">{events.map((event) => <article className="timeline-event" key={event.id}><span className={`timeline-dot ${levelClass(event.level)}`} /><div><div className="timeline-meta"><strong>{event.environment ?? "default"}</strong><time>{new Date(event.occurred_at).toLocaleString("zh-CN")}</time></div><p>{event.message}</p><div className="tag-list">{event.release && <span>release:{event.release}</span>}{Object.entries(event.tags ?? {}).slice(0, 4).map(([key, value]) => <span key={key}>{key}:{String(value)}</span>)}</div><details><summary>查看原始上下文</summary><pre>{JSON.stringify({ contexts: event.contexts, exception: event.exception }, null, 2)}</pre></details></div></article>)}{!events.length && <div className="small-empty">正在等待事件详情…</div>}</div></div><div className="drawer-section"><h3>团队协作</h3><div className="timeline">{comments.map(c=><article className="timeline-event" key={c.id}><span className="timeline-dot info"/><div><div className="timeline-meta"><strong>{c.author}</strong><time>{c.created_at&&new Date(c.created_at).toLocaleString("zh-CN")}</time></div><p>{c.body}</p></div></article>)}</div><form className="settings-form" onSubmit={addComment}><label>添加处理记录<textarea required maxLength={5000} value={commentBody} onChange={e=>setCommentBody(e.target.value)}/></label><button>发表评论</button></form></div><div className="drawer-section fingerprint"><h3>Fingerprint</h3>{canManage&&<button onClick={()=>changeIssueFingerprint(selected)}>调整</button>}<code>{selected.fingerprint}</code></div></aside>}
    </div>
  );
}
