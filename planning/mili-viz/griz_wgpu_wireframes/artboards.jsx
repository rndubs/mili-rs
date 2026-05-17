// Full-window App composition + small standalone studies.
// Each artboard renders <App {...props} /> or a study component.

const { useState: useStateA } = React;

// ---------- canonical transcript fragments ----------
const SAMPLE_USER_MSG = "why does state 47 show that hot spot on the upper brace?";
const SAMPLE_ASSISTANT = "Looks like sx peaks near 5.2e+04 Pa on brick 4213 at state 47 — about 3× the surrounding region. I'll scan for similar peaks across states and check whether it's a numerical spike or sustained loading.";

// tool-call rows — three densities reuse the same data
const SAMPLE_CALLS = [
  { kind: "ran",       sig: "state 47; show sx",                          result: "ok" },
  { kind: "queried",   sig: "sx range over states 40..60",                result: "[0, 5.2e+04]" },
  { kind: "queried",   sig: "elements where sx > 4e+04 at state 47",      result: "12 elements" },
  { kind: "captured",  sig: "frame  (state 47, view: front)",             result: "812 KB png" },
];

function ToolCallLine({ c }) {
  return (
    <div className="tc-line">
      <span className="tag">▸</span>
      <span>{c.kind}</span>
      <span>{c.sig}</span>
      {c.result && c.result !== "ok" && <><span className="arrow">→</span><span>{c.result}</span></>}
    </div>
  );
}
function ToolCallChip({ c }) {
  return (
    <span className="tc-chip"><span className="kind">{c.kind}</span><span className="val">{c.sig}</span></span>
  );
}
function ToolCallCard({ c }) {
  return (
    <div className="tc-card">
      <div className="head"><span>{c.kind} · {c.sig.split(/[(\s]/)[0]}</span><span className="ok" style={{color:"var(--ok)"}}>ok</span></div>
      <div className="args">{c.sig}</div>
      {c.result && c.result !== "ok" && <div className="result">→ {c.result}</div>}
    </div>
  );
}

// ---------- transcript renderer ----------
function Transcript({ density = "line", status = "idle", interrupted = false, showProvenanceInline = false }) {
  const Row = density === "card" ? ToolCallCard : density === "chip" ? ToolCallChip : ToolCallLine;
  const visibleCalls = status === "thinking" ? [] :
                       status === "running"  ? SAMPLE_CALLS.slice(0, 2) :
                       interrupted ? SAMPLE_CALLS.slice(0, 2) :
                       SAMPLE_CALLS;

  return (
    <div className="transcript">
      <div className="msg user">
        <div className="role">you · 2:14 pm</div>
        <div className="body">{SAMPLE_USER_MSG}</div>
      </div>
      {showProvenanceInline && (
        <div className="journal-row" style={{margin: "0 -10px", borderTop:"1px solid var(--border-soft)"}}>
          <span className="badge user"></span>
          <div>
            <div className="desc">— turn boundary —</div>
            <div className="meta">snapshot: state=47, result=sx, view=front</div>
          </div>
          <span className="meta" style={{color:"var(--accent)", cursor:"default"}}>↶ revert to here</span>
        </div>
      )}
      <div className="msg assistant">
        <div className="role">claude · {status === "running" ? "running…" : status === "thinking" ? "thinking…" : "2:14 pm"}</div>
        {density === "chip" ? (
          <div style={{margin: "4px 0"}}>{visibleCalls.map((c, i) => <ToolCallChip key={i} c={c} />)}</div>
        ) : (
          visibleCalls.map((c, i) => <Row key={i} c={c} />)
        )}
        {status === "thinking" && (
          <div className="tc-line" style={{color:"var(--text-faint)"}}><span className="tag">·</span> reading subscription stream…</div>
        )}
        {status === "running" && (
          <div className="tc-line" style={{color:"var(--warn)"}}>
            <span className="tag">▸</span> running: <span>scan sx for NaN/Inf across all states</span>
            <span style={{marginLeft: 6, color:"var(--text-faint)"}}>(38 / 96)</span>
          </div>
        )}
        {interrupted && (
          <div className="tc-line" style={{color:"var(--danger)"}}>
            <span className="tag">✕</span> interrupted by user — turn cancelled
          </div>
        )}
        {status === "idle" && !interrupted && (
          <div className="body" style={{marginTop: 6}}>{SAMPLE_ASSISTANT}</div>
        )}
      </div>
    </div>
  );
}

// ---------- journal tab content ----------
function JournalTab() {
  const rows = [
    { who: "agent", desc: "set state 47", meta: "2:14:08 pm · revertable" },
    { who: "agent", desc: "show sx", meta: "2:14:08 pm" },
    { who: "agent", desc: "captured frame", meta: "2:14:09 pm" },
    { who: "user",  desc: "— turn boundary —", meta: "2:14:04 pm" },
    { who: "agent", desc: "set view: front", meta: "2:12:42 pm" },
    { who: "user",  desc: "rotated camera", meta: "2:12:30 pm" },
    { who: "agent", desc: "load run d3samp6", meta: "2:11:55 pm" },
  ];
  return (
    <div style={{flex:1, overflow:"auto"}}>
      <div className="agent-banner">
        <span style={{color:"var(--text-dim)"}}>session changes · newest first</span>
        <button className="btn-ghost">↶ revert turn</button>
      </div>
      {rows.map((r, i) => (
        <div className="journal-row" key={i}>
          <span className={"badge " + r.who}></span>
          <div>
            <div className="desc">{r.who === "agent" ? "agent: " : "you: "}{r.desc}</div>
            <div className="meta">{r.meta}</div>
          </div>
          <span className="meta" style={{color: r.who === "agent" ? "var(--accent)" : "var(--text-faint)", cursor:"default"}}>
            {r.who === "agent" ? "↶ revert" : ""}
          </span>
        </div>
      ))}
    </div>
  );
}

// ---------- full AI panel ----------
function AIPanel({
  width = 320,
  status = "idle",
  interrupted = false,
  density = "line",
  showProvenanceInline = false,
  hasJournalTab = false,
  onCollapse,
}) {
  const [tab, setTab] = useStateA("chat");
  const running = status === "running";
  return (
    <div className="right-dock ai-panel" style={{width}}>
      <AIPanelHeader status={status} onCollapse={onCollapse} tab={tab} setTab={setTab} hasJournalTab={hasJournalTab} />
      {tab === "journal" && hasJournalTab ? (
        <JournalTab />
      ) : (
        <Transcript density={density} status={status} interrupted={interrupted} showProvenanceInline={showProvenanceInline} />
      )}
      <Composer
        attached={status === "thinking" ? ["frame · state 47"] : []}
        stopButton={running || status === "thinking"}
        running={running}
        placeholder={interrupted ? "follow up… (turn was interrupted)" : "ask…"}
      />
    </div>
  );
}

// ---------- focus-mode left rail ----------
function LeftRail() {
  return (
    <div className="left-dock" style={{width: 28, alignItems:"center"}}>
      <div style={{flex:1, display:"flex", flexDirection:"column", alignItems:"center", gap: 8, paddingTop: 6}}>
        {["R", "M", "S", "P"].map(k => (
          <div key={k} style={{
            width: 22, height: 22, display:"grid", placeItems:"center",
            border: "1px solid var(--border-soft)", borderRadius: "var(--radius)",
            background: "var(--panel-2)", color: "var(--text-dim)",
            fontFamily: "var(--font-mono)", fontSize: 10
          }}>{k}</div>
        ))}
      </div>
    </div>
  );
}

// ---------- timeline strip (provenance variant 3) ----------
function ProvenanceTimeline() {
  const events = [
    { t: "11:55", who: "agent", label: "load run" },
    { t: "12:30", who: "user",  label: "rotate" },
    { t: "12:42", who: "agent", label: "view front" },
    { t: "14:04", who: "user",  label: "ask" },
    { t: "14:08", who: "agent", label: "state 47" },
    { t: "14:08", who: "agent", label: "show sx" },
    { t: "14:09", who: "agent", label: "snapshot" },
  ];
  return (
    <div style={{
      display:"flex", alignItems:"center", gap: 10,
      padding: "4px 10px",
      background: "var(--panel)",
      borderBottom: "1px solid var(--border-soft)",
      fontFamily: "var(--font-mono)", fontSize: 10.5,
      color: "var(--text-dim)",
    }}>
      <span style={{color:"var(--text-faint)"}}>provenance →</span>
      {events.map((e, i) => (
        <span key={i} style={{display:"inline-flex", alignItems:"center", gap: 4}}>
          <span style={{
            width: 6, height: 6, borderRadius: "50%",
            background: e.who === "agent" ? "var(--accent)" : "var(--text-faint)"
          }}></span>
          <span style={{color: e.who === "agent" ? "var(--text)" : "var(--text-dim)"}}>{e.label}</span>
          {i < events.length - 1 && <span style={{color:"var(--border-soft)", marginLeft: 4}}>—</span>}
        </span>
      ))}
      <span style={{flex:1}}></span>
      <button className="btn-ghost" style={{height: 18, fontSize: 10}}>↶ revert turn</button>
    </div>
  );
}

// ---------- main app composition ----------
function App({
  // layout
  showLeftDock = true,
  leftDockWidth = 230,
  leftRailOnly = false,
  showRightAI = false,
  aiPanelWidth = 320,
  showBottomTabs = true,
  bottomTabsActive = "cmd",
  bottomTabsHeight = 200,
  // viewport / session
  attached = true,
  overlays: overlaysProp = { title: true, state: true, legend: true, axes: true, bbox: true },
  animating = false,
  peers = [],
  // AI panel state
  agentStatus = "idle",
  interrupted = false,
  density = "line",
  showProvenanceInline = false,
  hasJournalTab = false,
  showProvenanceTimeline = false,
  // misc
  theme = "dark",
}) {
  const [overlays, setOverlays] = useStateA(overlaysProp);
  const setOverlay = (k, v) => setOverlays(prev => ({ ...prev, [k]: v }));
  // grid template for body
  let cols;
  if (leftRailOnly) {
    cols = `28px 1fr ${showRightAI ? aiPanelWidth + "px" : "28px"}`;
  } else {
    cols = `${showLeftDock ? leftDockWidth + "px" : "0px"} 1fr ${showRightAI ? aiPanelWidth + "px" : "28px"}`;
  }

  return (
    <div className={"app theme-" + theme}>
      <MenuBar />
      <Toolbar animating={animating} overlays={overlays} setOverlay={setOverlay} />
      <div className="body" style={{gridTemplateColumns: cols, gridTemplateRows: showProvenanceTimeline ? "auto 1fr" : "1fr"}}>
        {showProvenanceTimeline && (
          <div style={{gridColumn:"1 / -1"}}>
            <ProvenanceTimeline />
          </div>
        )}
        {leftRailOnly ? <LeftRail /> : (showLeftDock && <LeftDock width={leftDockWidth} attached={attached} />)}
        <Viewport
          overlays={overlays}
          empty={!attached}
          animating={animating}
          peers={peers}
        />
        {showRightAI ? (
          <AIPanel
            width={aiPanelWidth}
            status={agentStatus}
            interrupted={interrupted}
            density={density}
            showProvenanceInline={showProvenanceInline}
            hasJournalTab={hasJournalTab}
          />
        ) : (
          <AIRail />
        )}
      </div>
      {showBottomTabs && <BottomTabs active={bottomTabsActive} height={bottomTabsHeight} />}
      <StatusBar
        attached={attached ? "d3samp6@hpc-login01" : "— not attached —"}
        pick={attached ? "brick 4213" : "—"}
        peers={peers.length}
      />
    </div>
  );
}

// ---------- AI-panel-only studies ----------
// Sized to live inside a small artboard. Provide the chrome around so the
// panel reads as in-context.
function AIStudyFrame({ children, title, width = 380, height = 520 }) {
  return (
    <div style={{
      width, height,
      display:"grid", gridTemplateRows: "auto 1fr",
      background: "var(--bg)",
      fontFamily:"var(--font-sans)", color:"var(--text)", fontSize:"var(--fs)",
    }} className="app theme-dark">
      <div style={{
        padding: "6px 10px", borderBottom: "1px solid var(--border-soft)",
        background: "var(--panel)", color: "var(--text-dim)", fontSize: 11,
        letterSpacing: "0.04em", textTransform:"uppercase",
      }}>{title}</div>
      <div style={{display:"grid", minHeight: 0}}>{children}</div>
    </div>
  );
}

function ToolCallStudy({ density }) {
  return (
    <AIStudyFrame title={`tool-call density · ${density}`}>
      <AIPanel width="100%" status="idle" density={density} />
    </AIStudyFrame>
  );
}

function ProvenanceStudy({ variant }) {
  if (variant === "inline") {
    return (
      <AIStudyFrame title="provenance · inline at turn boundary">
        <AIPanel width="100%" status="idle" density="line" showProvenanceInline />
      </AIStudyFrame>
    );
  }
  if (variant === "journal") {
    return (
      <AIStudyFrame title="provenance · journal tab">
        <AIPanel width="100%" status="idle" density="line" hasJournalTab />
      </AIStudyFrame>
    );
  }
  return null; // timeline variant uses full App
}

Object.assign(window, { App, AIPanel, ToolCallStudy, ProvenanceStudy, ProvenanceTimeline });
