import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ReactNode, useEffect, useRef, useState } from "react";
import { api, emptyProfile, LogLine, Profile, Status } from "./api";
import { Back, Chevron, Close, Dots, Globe, ShieldAlert, ShieldBusy, ShieldCheck, Terminal } from "./icons";

const MAX_LOGS = 1000;

type View = "main" | "settings" | "logs";

function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

function duration(seconds: number) {
  const s = Math.max(0, Math.floor(seconds));
  const pad = (n: number) => n.toString().padStart(2, "0");
  return `${Math.floor(s / 3600)}:${pad(Math.floor((s % 3600) / 60))}:${pad(s % 60)}`;
}

function useNow(enabled: boolean) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!enabled) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [enabled]);
  return now;
}

/** Closes a popup when clicking anywhere outside it. */
function useDismiss(open: boolean, close: () => void) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) close();
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open, close]);
  return ref;
}

export default function App() {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [draft, setDraft] = useState<Profile>(emptyProfile());
  const [status, setStatus] = useState<Status>({ state: "disconnected" });
  const [logs, setLogs] = useState<LogLine[]>([]);
  const [view, setView] = useState<View>("main");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const passwordRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    api.listProfiles().then((list) => {
      setProfiles(list);
      if (list.length) setDraft({ ...list[0] });
    });
    api.getStatus().then(setStatus);
    api.getLogs().then(setLogs);

    const unlisten = [
      listen<Status>("status", (e) => setStatus(e.payload)),
      listen<LogLine>("log", (e) => setLogs((prev) => [...prev.slice(-(MAX_LOGS - 1)), e.payload])),
      listen<string>("request-password", (e) => {
        api.listProfiles().then((list) => {
          const p = list.find((x) => x.id === e.payload);
          if (p) setDraft({ ...p });
        });
        setView("main");
        setError("Enter your password to connect.");
        setTimeout(() => passwordRef.current?.focus(), 50);
      }),
    ];
    return () => unlisten.forEach((p) => p.then((f) => f()));
  }, []);

  // While a connection is up, show the profile it belongs to.
  useEffect(() => {
    if (status.state !== "connecting" && status.state !== "connected") return;
    const p = profiles.find((x) => x.id === status.profileId);
    if (p && p.id !== draft.id) setDraft({ ...p });
  }, [status, profiles, draft.id]);

  const idle = status.state === "disconnected" || status.state === "error";
  const now = useNow(status.state === "connected");

  // Whether this connection has saved sign-in cookies; re-checked after each connect attempt.
  const [hasCredentials, setHasCredentials] = useState(false);
  useEffect(() => {
    if (!draft.id) return setHasCredentials(false);
    api.hasSavedSignIn(draft.id).then(setHasCredentials, () => setHasCredentials(false));
  }, [draft.id, status.state]);

  const deleteCredentials = () =>
    run(async () => {
      await api.forgetSignIn(draft.id);
      setHasCredentials(false);
    });

  async function run(action: () => Promise<unknown>) {
    setError(null);
    setBusy(true);
    try {
      await action();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  /** Saves the draft and returns the stored copy (with its id). */
  async function persist(p: Profile): Promise<Profile> {
    const toSave = { ...p, name: p.name.trim() || p.portal.trim() };
    const list = await api.saveProfile(toSave);
    setProfiles(list);
    const saved = toSave.id ? list.find((x) => x.id === toSave.id)! : list[list.length - 1];
    setDraft({ ...saved });
    return saved;
  }

  const connect = () =>
    run(async () => {
      if (!draft.portal.trim()) throw "Enter a server address";
      const saved = await persist(draft);
      await api.connect(saved.id, saved.auth === "password" ? password : undefined);
      setPassword("");
    });

  const selectProfile = (p: Profile | null) => {
    setDraft(p ? { ...p } : emptyProfile());
    setPassword("");
    setError(null);
  };

  /** Removes a saved connection; if it was the one shown, switch to another. */
  const removeProfile = (p: Profile) =>
    run(async () => {
      const list = await api.deleteProfile(p.id);
      setProfiles(list);
      if (p.id === draft.id) selectProfile(list[0] ?? null);
    });

  const deleteProfile = () => {
    if (!draft.id) return selectProfile(profiles[0] ?? null);
    removeProfile(draft);
    setView("main");
  };

  return (
    <div className="app">
      <header data-tauri-drag-region>
        {view !== "main" ? (
          <button className="icon-btn" onClick={() => setView("main")} aria-label="Back">
            <Back />
          </button>
        ) : (
          <span className="icon-btn placeholder" />
        )}
        <h1 data-tauri-drag-region>{view === "settings" ? "Settings" : view === "logs" ? "Logs" : "GlobalProtect"}</h1>
        <button className="icon-btn round" onClick={() => getCurrentWindow().hide()} aria-label="Close">
          <Close />
        </button>
      </header>

      {view === "main" && (
        <main className={`main-view${idle && draft.auth === "password" ? " compact" : ""}`}>
          <div className="toolbar">
            <Menu
              items={[
                { label: "Connection settings", onClick: () => setView("settings") },
                { label: "New connection", onClick: () => selectProfile(null), disabled: !idle },
                { label: "Delete credentials", onClick: deleteCredentials, disabled: !hasCredentials, danger: true },
                { label: "Delete connection", onClick: deleteProfile, disabled: !idle || !draft.id, danger: true },
                { label: "Logs", onClick: () => setView("logs") },
                { label: "Quit", onClick: () => api.quit() },
                { label: "⚠️ Vibe coded: use at your own risk", onClick: () => setView("logs"), warning: true },
              ]}
            />
            <button className="icon-btn" onClick={() => setView("logs")} aria-label="Logs" title="Logs">
              <Terminal />
            </button>
          </div>

          <div className={`shield ${status.state}`}>
            {status.state === "connected" ? (
              <ShieldCheck size={58} />
            ) : status.state === "connecting" || status.state === "disconnecting" ? (
              <ShieldBusy size={58} />
            ) : (
              <ShieldAlert size={58} />
            )}
          </div>

          <div className="headline">{headline(status)}</div>

          <ProfilePicker
            profiles={profiles}
            current={draft}
            disabled={!idle}
            onSelect={selectProfile}
            onRemove={removeProfile}
          />

          <div className="field-head">
            <span>Server address</span>
            <TypePicker
              asGateway={draft.asGateway}
              disabled={!idle}
              onChange={(asGateway) => setDraft({ ...draft, asGateway })}
            />
          </div>
          <AddressInput
            value={draft.portal}
            disabled={!idle}
            profiles={profiles}
            onChange={(portal) => setDraft({ ...draft, portal })}
            onPick={selectProfile}
            onRemove={removeProfile}
            onEnter={connect}
          />

          {idle && draft.auth === "password" && (
            <input
              ref={passwordRef}
              className="text-input"
              type="password"
              placeholder={`Password${draft.username ? ` for ${draft.username}` : ""}`}
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && connect()}
              autoComplete="current-password"
            />
          )}

          {idle ? (
            <button className="primary" disabled={busy} onClick={connect}>
              Connect
            </button>
          ) : status.state === "connecting" ? (
            <button className="primary muted" disabled={busy} onClick={() => run(api.disconnect)}>
              Cancel
            </button>
          ) : (
            <button className="primary danger" disabled={busy || status.state === "disconnecting"} onClick={() => run(api.disconnect)}>
              Disconnect
            </button>
          )}

          {(error || status.state === "error") && (
            <div className="error-text" title={error ?? (status.state === "error" ? status.message : "")}>
              {error ?? (status.state === "error" ? status.message : "")}
            </div>
          )}

          {!error && status.state !== "error" && !(idle && draft.auth === "password") && <StatusPill status={status} now={now} profile={draft} />}
        </main>
      )}

      {view === "settings" && (
        <Settings
          profile={draft}
          disabled={!idle || busy}
          onChange={setDraft}
          onSave={() =>
            run(async () => {
              if (!draft.portal.trim()) throw "Enter a server address";
              await persist(draft);
              setView("main");
            })
          }
          error={error}
        />
      )}

      {view === "logs" && <LogView logs={logs} onClear={() => api.clearLogs().then(() => setLogs([]))} />}
    </div>
  );
}

function headline(status: Status) {
  switch (status.state) {
    case "connected":
      return "Connected";
    case "connecting":
      return "Connecting…";
    case "disconnecting":
      return "Disconnecting…";
    case "error":
      return "Connection Failed";
    default:
      return "Not Connected";
  }
}

function StatusPill({ status, now, profile }: { status: Status; now: number; profile: Profile }) {
  let text: string;
  switch (status.state) {
    case "connected":
      text = `Connected · ${duration(now / 1000 - status.since)}`;
      break;
    case "connecting":
      text = "Signing in…";
      break;
    case "disconnecting":
      text = "Disconnecting…";
      break;
    case "error":
      text = "Last attempt failed";
      break;
    default:
      text =
        profile.auth === "password"
          ? "Password sign-in"
          : profile.auth === "browser"
            ? "Browser sign-in"
            : "SAML sign-in";
  }
  return (
    <div className="pill status-pill">
      <span className={`dot ${status.state}`} />
      {text}
    </div>
  );
}

interface MenuItem {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  danger?: boolean;
  warning?: boolean;
}

function Menu({ items }: { items: MenuItem[] }) {
  const [open, setOpen] = useState(false);
  const ref = useDismiss(open, () => setOpen(false));
  return (
    <div className="popover-anchor" ref={ref}>
      <button className="icon-btn" onClick={() => setOpen(!open)} aria-label="Menu">
        <Dots />
      </button>
      {open && (
        <div className="popover left">
          {items.map((item) => (
            <button
              key={item.label}
              className={item.danger ? "danger-text" : item.warning ? "vibe-item" : ""}
              disabled={item.disabled}
              onClick={() => {
                setOpen(false);
                item.onClick();
              }}
            >
              {item.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function Dropdown({
  trigger,
  className,
  align = "center",
  disabled,
  children,
}: {
  trigger: ReactNode;
  className: string;
  align?: "center" | "right";
  disabled?: boolean;
  children: (close: () => void) => ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const ref = useDismiss(open, () => setOpen(false));
  return (
    <div className="popover-anchor" ref={ref}>
      <button className={className} disabled={disabled} onClick={() => setOpen(!open)}>
        {trigger}
      </button>
      {open && <div className={`popover ${align}`}>{children(() => setOpen(false))}</div>}
    </div>
  );
}

/** A dropdown entry with a hover "×" that removes it without selecting it. */
function RemovableRow({ label, onRemove, children }: { label: string; onRemove: () => void; children: ReactNode }) {
  return (
    <div className="item-row">
      {children}
      <button
        className="remove"
        title={`Remove ${label}`}
        aria-label={`Remove ${label}`}
        onMouseDown={(e) => e.stopPropagation()}
        onClick={(e) => {
          e.stopPropagation();
          onRemove();
        }}
      >
        <Close size={12} />
      </button>
    </div>
  );
}

function ProfilePicker({
  profiles,
  current,
  disabled,
  onSelect,
  onRemove,
}: {
  profiles: Profile[];
  current: Profile;
  disabled: boolean;
  onSelect: (p: Profile | null) => void;
  onRemove: (p: Profile) => void;
}) {
  const label = current.id ? current.name : current.portal.trim() || "New connection";
  return (
    <Dropdown
      className="pill picker"
      disabled={disabled}
      trigger={
        <>
          <span className="ellipsis">{label}</span>
          <Chevron />
        </>
      }
    >
      {(close) => (
        <>
          {profiles.map((p) => (
            <RemovableRow key={p.id} label={p.name} onRemove={() => onRemove(p)}>
              <button
                className={p.id === current.id ? "selected" : ""}
                onClick={() => {
                  close();
                  onSelect(p);
                }}
              >
                {p.name}
              </button>
            </RemovableRow>
          ))}
          {profiles.length > 0 && <hr />}
          <button
            onClick={() => {
              close();
              onSelect(null);
            }}
          >
            + New connection
          </button>
        </>
      )}
    </Dropdown>
  );
}

function TypePicker({
  asGateway,
  disabled,
  onChange,
}: {
  asGateway: boolean;
  disabled: boolean;
  onChange: (asGateway: boolean) => void;
}) {
  return (
    <Dropdown
      className="type-picker"
      align="right"
      disabled={disabled}
      trigger={
        <>
          <Globe />
          {asGateway ? "Gateway" : "Portal"}
          <Chevron size={12} />
        </>
      }
    >
      {(close) =>
        [false, true].map((g) => (
          <button
            key={String(g)}
            className={g === asGateway ? "selected" : ""}
            onClick={() => {
              close();
              onChange(g);
            }}
          >
            {g ? "Gateway" : "Portal"}
          </button>
        ))
      }
    </Dropdown>
  );
}

function AddressInput({
  value,
  disabled,
  profiles,
  onChange,
  onPick,
  onRemove,
  onEnter,
}: {
  value: string;
  disabled: boolean;
  profiles: Profile[];
  onChange: (v: string) => void;
  onPick: (p: Profile) => void;
  onRemove: (p: Profile) => void;
  onEnter: () => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useDismiss(open, () => setOpen(false));
  return (
    <div className="address popover-anchor" ref={ref}>
      <input
        className="text-input"
        value={value}
        disabled={disabled}
        placeholder="vpn.example.com"
        spellCheck={false}
        autoCapitalize="off"
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && onEnter()}
      />
      {profiles.length > 0 && (
        <button className="address-toggle" disabled={disabled} onClick={() => setOpen(!open)} aria-label="Recent servers">
          <Chevron size={18} />
        </button>
      )}
      {open && profiles.length > 0 && (
        <div className="popover full">
          {profiles.map((p) => (
            <RemovableRow key={p.id} label={p.portal} onRemove={() => onRemove(p)}>
              <button
                onClick={() => {
                  setOpen(false);
                  onPick(p);
                }}
              >
                {p.portal}
                {p.name !== p.portal && <span className="sub">{p.name}</span>}
              </button>
            </RemovableRow>
          ))}
        </div>
      )}
    </div>
  );
}

function Settings({
  profile,
  disabled,
  onChange,
  onSave,
  error,
}: {
  profile: Profile;
  disabled: boolean;
  onChange: (p: Profile) => void;
  onSave: () => void;
  error: string | null;
}) {
  const set = <K extends keyof Profile>(key: K, value: Profile[K]) => onChange({ ...profile, [key]: value });
  return (
    <form
      className="settings"
      onSubmit={(e) => {
        e.preventDefault();
        onSave();
      }}
    >
      <fieldset disabled={disabled}>
        <label>
          Name
          <input className="text-input" value={profile.name} placeholder={profile.portal || "Work"} onChange={(e) => set("name", e.target.value)} />
        </label>
        <label>
          Server address
          <input className="text-input" value={profile.portal} placeholder="vpn.example.com" onChange={(e) => set("portal", e.target.value)} />
        </label>
        {!profile.asGateway && (
          <label>
            Gateway
            <input
              className="text-input"
              value={profile.gateway}
              placeholder="Automatic"
              onChange={(e) => set("gateway", e.target.value)}
            />
          </label>
        )}
        <label>
          Sign-in method
          <select className="text-input" value={profile.auth} onChange={(e) => set("auth", e.target.value as Profile["auth"])}>
            <option value="embedded">SAML (built-in window)</option>
            <option value="browser">SAML (system browser)</option>
            <option value="password">Username and password</option>
          </select>
        </label>
        {profile.auth === "browser" && (
          <label>
            Browser
            <input
              className="text-input"
              value={profile.browser}
              placeholder="auto"
              onChange={(e) => set("browser", e.target.value)}
            />
          </label>
        )}
        {profile.auth === "password" && (
          <label>
            Username
            <input className="text-input" value={profile.username} onChange={(e) => set("username", e.target.value)} />
          </label>
        )}
        <label className="check">
          <input type="checkbox" checked={profile.hip} onChange={(e) => set("hip", e.target.checked)} />
          Send HIP reports
        </label>
        <SignInMemory profile={profile} onToggle={(on) => set("rememberSignIn", on)} />
        <label className="check">
          <input type="checkbox" checked={profile.ignoreTlsErrors} onChange={(e) => set("ignoreTlsErrors", e.target.checked)} />
          Ignore TLS errors (insecure)
        </label>
      </fieldset>
      {error && <div className="error-text">{error}</div>}
      <button className="primary" type="submit" disabled={disabled}>
        Save
      </button>
      <VibeCoded />
    </form>
  );
}

/** "Remember sign-in" toggle, plus a way to drop a saved sign-in. */
function SignInMemory({ profile, onToggle }: { profile: Profile; onToggle: (on: boolean) => void }) {
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    if (!profile.id) return setSaved(false);
    api.hasSavedSignIn(profile.id).then(setSaved, () => setSaved(false));
  }, [profile.id]);

  return (
    <div className="sign-in-memory">
      <label className="check">
        <input type="checkbox" checked={profile.rememberSignIn} onChange={(e) => onToggle(e.target.checked)} />
        Remember sign-in
      </label>
      <div className="sub">
        {profile.asGateway
          ? "Not used when connecting directly to a gateway."
          : saved
            ? "Signed in. Connecting won't ask you to log in until the portal expires it."
            : "After you log in once, later connections skip the login page."}
      </div>
      {saved && (
        <button
          type="button"
          className="link-btn danger-text"
          onClick={() => api.forgetSignIn(profile.id).then(() => setSaved(false))}
        >
          Delete credentials
        </button>
      )}
    </div>
  );
}

/** Shown wherever there's room: this app was AI-written and not audited. */
function VibeCoded() {
  return (
    <div className="vibe">
      <strong>⚠️ VIBE CODED</strong>
      <span>
        Written almost entirely by an AI (Claude) with minimal human review. It handles VPN credentials and drives a
        root service. Use at your own risk; no warranty.
      </span>
    </div>
  );
}

function LogView({ logs, onClear }: { logs: LogLine[]; onClear: () => void }) {
  const ref = useRef<HTMLPreElement>(null);
  useEffect(() => {
    ref.current?.scrollTo({ top: ref.current.scrollHeight });
  }, [logs]);
  return (
    <div className="logs">
      <pre ref={ref} className="log-view">
        {logs.length === 0 && <span className="sub">No log output yet.</span>}
        {logs.map((l, i) => (
          <div key={i} className={`log-${l.level.toLowerCase()}`}>
            {new Date(l.time * 1000).toLocaleTimeString()} {l.message}
          </div>
        ))}
      </pre>
      <VibeCoded />
      <button className="pill" onClick={onClear} disabled={logs.length === 0}>
        Clear
      </button>
    </div>
  );
}
