// Shared egui-flavored chrome pieces. Pure presentational — each artboard
// composes these into a full window.
const { useState } = React;

// ---------- menu bar ----------
const MENU_ITEMS = ["Control", "Rendering", "Picking", "Results", "Time", "Plot", "Help"];
function MenuBar() {
  return (
    <div className="menubar">
      {MENU_ITEMS.map(m => <div className="menu" key={m}>{m}</div>)}
    </div>
  );
}

// ---------- toolbar ----------
const OVERLAY_KEYS = [
  { k: "title",  label: "title"  },
  { k: "state",  label: "state"  },
  { k: "legend", label: "legend" },
  { k: "axes",   label: "axes"   },
  { k: "bbox",   label: "bbox"   },
];
function Toolbar({ animating = false, state = 47, totalStates = 96, overlays, setOverlay }) {
  return (
    <div className="toolbar">
      <div className="group">
        <button className="btn flat" title="first">⏮</button>
        <button className="btn flat" title="prev">◀</button>
        <button className="btn flat" title="next">▶</button>
        <button className="btn flat" title="last">⏭</button>
      </div>
      <div className="sep"></div>
      <div className="group">
        <span className="label">stride</span>
        <div className="stride"><input defaultValue="1" /></div>
      </div>
      <div className="sep"></div>
      <div className="group">
        <button className={"btn " + (animating ? "active" : "")}>{animating ? "⏸ pause" : "▶ animate"}</button>
        <button className="btn flat">⏹</button>
      </div>
      <div className="sep"></div>
      <div className="group">
        <button className="btn flat">⟲ view reset</button>
        <button className="btn flat">⊞ fit</button>
      </div>
      <div className="sep"></div>
      <div className="group">
        <span className="label">overlays</span>
        {OVERLAY_KEYS.map(({k, label}) => (
          <button
            key={k}
            className={"btn " + (overlays && overlays[k] ? "active" : "flat")}
            title={`toggle ${label}`}
            onClick={() => setOverlay && setOverlay(k, !(overlays && overlays[k]))}
          >{label}</button>
        ))}
      </div>
      <div style={{flex: 1}}></div>
      <div className="group">
        <span className="label" style={{fontFamily: "var(--font-mono)"}}>
          state <span style={{color: "var(--text)"}}>{state}</span>
          <span style={{color: "var(--text-faint)"}}> / {totalStates}</span>
        </span>
      </div>
    </div>
  );
}

// ---------- left dock ----------
function Section({ title, meta, defaultOpen = true, children }) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="section">
      <div className="section-header" onClick={() => setOpen(!open)}>
        <span className="chev">{open ? "▼" : "▶"}</span>
        <span className="title">{title}</span>
        {meta && <span className="meta">{meta}</span>}
      </div>
      {open && <div className="section-body">{children}</div>}
    </div>
  );
}

function TreeRow({ glyph = "·", name, status, selected, muted }) {
  return (
    <div className={"tree-row " + (selected ? "selected " : "") + (muted ? "muted" : "")}>
      <span className="glyph">{glyph}</span>
      <span className="name">{name}</span>
      {status === "active" && <span className="dot"></span>}
      {status === "warn" && <span className="dot warn"></span>}
      {status === "dim" && <span className="dot dim"></span>}
    </div>
  );
}

function LeftDock({ width = 230, attached = true }) {
  if (!attached) {
    return (
      <div className="left-dock" style={{ width }}>
        <div className="dock-header"><span>Sessions</span></div>
        <div className="dock-body" style={{padding: "12px 10px", color: "var(--text-faint)", fontSize: 11}}>
          No session attached.
        </div>
      </div>
    );
  }
  return (
    <div className="left-dock" style={{ width }}>
      <div className="dock-header"><span>Session</span><span style={{color:"var(--text-faint)"}}>v1</span></div>
      <div className="dock-body">
        <Section title="Runs / sessions" defaultOpen>
          <TreeRow glyph="▣" name="d3samp6" status="active" selected />
          <TreeRow glyph="▢" name="run-2024-09-12" muted />
          <TreeRow glyph="+ " name="attach…" muted />
        </Section>
        <Section title="Results" meta="142">
          <TreeRow glyph="▾" name="derived" />
          <div style={{paddingLeft: 14}}>
            <TreeRow glyph="·" name="sx (stress xx)" selected status="active" />
            <TreeRow glyph="·" name="sy" />
            <TreeRow glyph="·" name="von Mises" />
            <TreeRow glyph="·" name="pressure" />
          </div>
          <TreeRow glyph="▸" name="primal" />
          <TreeRow glyph="▸" name="time-indep" muted />
        </Section>
        <Section title="Materials" meta="8">
          <TreeRow glyph="◼" name="brick" status="active" />
          <TreeRow glyph="◼" name="shell" status="active" />
          <TreeRow glyph="◻" name="beam" status="dim" muted />
          <TreeRow glyph="◼" name="pad-1" status="active" />
        </Section>
        <Section title="Surfaces" meta="3" defaultOpen={false}>
          <TreeRow glyph="◻" name="surf-1" muted />
        </Section>
      </div>
    </div>
  );
}

// ---------- viewport ----------
function Viewport({
  overlays = { title: true, state: true, legend: true, axes: true, bbox: true },
  showOverlayToggle = true,
  modelLabel,
  banner = null,
  empty = false,
  animating = false,
  peers = [],
}) {
  if (empty) {
    return (
      <div className="viewport-wrap">
        <div className="viewport-bg"></div>
        <div className="empty">
          <div className="title">No session attached</div>
          <div className="hint">Connect to a <span style={{fontFamily:"var(--font-mono)"}}>mili-viz-server</span> to begin.</div>
          <div className="attach-card">
            <div style={{color: "var(--text)", fontSize: 12}}>Attach to session</div>
            <div style={{display:"flex", gap: 6}}>
              <button className="btn-ghost" style={{height: 24}}>Open session file…</button>
              <button className="btn-primary" style={{height: 24}}>Connect host…</button>
            </div>
            <div className="hint" style={{marginTop:4}}>recent: <span style={{color:"var(--text-dim)"}}>hpc-login01:7421</span></div>
          </div>
        </div>
      </div>
    );
  }
  return (
    <div className="viewport-wrap">
      <div className="viewport-bg"></div>
      {overlays.bbox && <div className="overlay bbox"></div>}
      <div className="viewport-model">
        <div className="shape"></div>
      </div>
      {banner}
      {peers.length > 0 && (
        <div className="peer-banner">
          {peers.map((p, i) => (
            <span key={i} style={{display:"inline-flex", gap: 4, alignItems:"center"}}>
              <span className="peer-dot" style={{background: p.color}}></span>{p.name}
            </span>
          ))}
          <span style={{color:"var(--text-faint)"}}>· viewing</span>
        </div>
      )}
      {overlays.title && (
        <>
          <div className="overlay title">d3samp6 · sx (stress xx)</div>
          <div className="overlay subtitle">elements: 12,488  ·  nodes: 14,210</div>
        </>
      )}
      {overlays.state && (
        <div className="overlay state">
          state 47 / 96{animating ? "  ▶" : ""}<br/>
          t = 4.7000e-03 s
        </div>
      )}
      {overlays.legend && (
        <div className="overlay legend">
          <div style={{display:"flex", gap: 6}}>
            <div className="bar"></div>
            <div className="scale" style={{display:"flex", flexDirection:"column", justifyContent:"space-between", padding:"1px 0"}}>
              <span>5.2e+04</span>
              <span>3.9e+04</span>
              <span>2.6e+04</span>
              <span>1.3e+04</span>
              <span>0.0e+00</span>
            </div>
          </div>
          <div style={{fontSize: 9, color:"rgba(220,225,232,0.6)"}}>sx [Pa]</div>
        </div>
      )}
      {overlays.axes && (
        <div className="overlay axes">
          <svg viewBox="0 0 56 56">
            <line x1="28" y1="28" x2="48" y2="34" stroke="rgba(230,90,90,0.9)" strokeWidth="1.5"/>
            <line x1="28" y1="28" x2="34" y2="8" stroke="rgba(120,200,120,0.9)" strokeWidth="1.5"/>
            <line x1="28" y1="28" x2="14" y2="44" stroke="rgba(120,160,230,0.9)" strokeWidth="1.5"/>
            <text x="49" y="36" fontSize="8" fill="rgba(230,90,90,0.95)" fontFamily="monospace">x</text>
            <text x="35" y="9" fontSize="8" fill="rgba(120,200,120,0.95)" fontFamily="monospace">y</text>
            <text x="9" y="46" fontSize="8" fill="rgba(120,160,230,0.95)" fontFamily="monospace">z</text>
          </svg>
        </div>
      )}
      {modelLabel && (
        <div className="overlay" style={{bottom: 12, left: "50%", transform: "translateX(-50%)", color:"var(--text-faint)"}}>
          {modelLabel}
        </div>
      )}
    </div>
  );
}

// ---------- AI rail (collapsed) ----------
function AIRail({ onExpand }) {
  return (
    <div className="ai-rail">
      <div className="pill" onClick={onExpand}>AI ASSISTANT</div>
      <div style={{flex: 1}}></div>
      <div style={{fontFamily:"var(--font-mono)", color:"var(--text-faint)", fontSize: 9, writingMode:"vertical-rl", transform:"rotate(180deg)"}}>idle</div>
    </div>
  );
}

// ---------- AI panel ----------
function AIPanelHeader({ status = "idle", onCollapse, tab, setTab, hasJournalTab }) {
  const label = { idle: "idle", thinking: "thinking", running: "running", interrupted: "interrupted" }[status] || "idle";
  return (
    <>
      <div className="ai-header">
        <span>AI ASSISTANT</span>
        <span style={{display:"inline-flex", gap: 10, alignItems:"center"}}>
          <span className={"agent-status " + status}><span className="dot"></span>{label}</span>
          <span style={{color:"var(--text-faint)", cursor:"default"}} onClick={onCollapse} title="collapse">›</span>
        </span>
      </div>
      {hasJournalTab && (
        <div className="ai-tabs">
          <div className={"tab " + (tab === "chat" ? "active" : "")} onClick={() => setTab("chat")}>Chat</div>
          <div className={"tab " + (tab === "journal" ? "active" : "")} onClick={() => setTab("journal")}>Journal</div>
        </div>
      )}
    </>
  );
}

// ---------- composer ----------
function Composer({ attached = ["frame · state 47"], placeholder = "ask…", stopButton = false, running = false }) {
  return (
    <div className="composer">
      {attached.length > 0 && (
        <div className="attach-row">
          {attached.map((a, i) => (
            <span key={i} className="attach">📷 {a} <span style={{color:"var(--text-faint)"}}>×</span></span>
          ))}
        </div>
      )}
      <div className="input">
        <span className="placeholder">{placeholder}</span>
      </div>
      <div className="actions">
        <div className="left">
          <button className="btn-ghost" title="attach current frame">📷</button>
          <button className="btn-ghost" title="run code">⌨</button>
        </div>
        <div className="right">
          {stopButton ? (
            <button className="btn-danger">⏹ Stop</button>
          ) : (
            <button className="btn-primary" disabled={running}>{running ? "running…" : "Send ↵"}</button>
          )}
        </div>
      </div>
    </div>
  );
}

// ---------- bottom tabs ----------
function BottomTabs({ active = "cmd", height = 200 }) {
  return (
    <div className="bottom" style={{height}}>
      <div className="tabs">
        <div className={"tab " + (active === "cmd" ? "active" : "")}>Command line</div>
        <div className={"tab " + (active === "py" ? "active" : "")}>Scripting</div>
        <div className={"tab " + (active === "plot" ? "active" : "")}>Time-history plot</div>
        <div style={{flex:1}}></div>
        <div className="tab" style={{color:"var(--text-faint)"}}>—</div>
      </div>
      <div className="tab-body">
        {active === "cmd" && (
          <>
            <div className="line"><span style={{color:"var(--text-faint)"}}>griz&gt;</span> <span className="echo">state 47</span></div>
            <div className="line" style={{color:"var(--text-faint)"}}>  state set to 47 / 96  t=4.7000e-03</div>
            <div className="line"><span style={{color:"var(--text-faint)"}}>griz&gt;</span> <span className="echo">show sx</span></div>
            <div className="line" style={{color:"var(--text-faint)"}}>  result: derived/sx · range [0.00, 5.20e+04]</div>
            <div className="line"><span className="prompt">griz&gt;</span> <span style={{color:"var(--text-faint)"}}>_</span></div>
          </>
        )}
        {active === "py" && (
          <>
            <div className="line"><span style={{color:"var(--text-faint)"}}># snippet.py — attached to d3samp6</span></div>
            <div className="line"><span style={{color:"var(--accent)"}}>import</span> griz <span style={{color:"var(--accent)"}}>as</span> g</div>
            <div className="line">s = g.attach()</div>
            <div className="line">s.state(<span style={{color:"var(--warn)"}}>47</span>); s.show(<span style={{color:"var(--ok)"}}>"sx"</span>)</div>
            <div className="line" style={{color:"var(--text-faint)", marginTop: 6}}>► Run · venv: griz-0.4.2 · attach: session-9f3a</div>
          </>
        )}
        {active === "plot" && (
          <div style={{height: "100%", display:"flex", alignItems:"center", justifyContent:"center", color:"var(--text-faint)"}}>
            [ egui_plot — sx vs. time, brick #4213 ]
          </div>
        )}
      </div>
    </div>
  );
}

// ---------- status bar ----------
function StatusBar({ attached = "d3samp6@hpc-login01", pick = "brick 4213", peers = 0 }) {
  return (
    <div className="statusbar">
      <span className="seg"><span className="dot"></span>attached <span style={{color:"var(--text)"}}>{attached}</span></span>
      <span className="seg" style={{color:"var(--text-faint)"}}>proto v1</span>
      <span className="seg">pick: <span style={{color:"var(--text)"}}>{pick}</span></span>
      <span className="spacer"></span>
      {peers > 0 && <span className="seg"><span className="dot warn"></span>{peers} peer{peers > 1 ? "s" : ""}</span>}
      <span className="seg" style={{color:"var(--text-faint)"}}>fps 58</span>
    </div>
  );
}

// expose
Object.assign(window, {
  MenuBar, Toolbar, Section, TreeRow, LeftDock, Viewport,
  AIRail, AIPanelHeader, Composer, BottomTabs, StatusBar
});
