import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import icon from "../src-tauri/icons/32x32.png";
import "./App.css";

type ButtonSetting = { isEnabled: boolean; color: string };
type Settings = {
  enableClickCircle: boolean;
  enableHoverDot: boolean;
  indicatorSize: number;
  hoverDotSize: number;
  hoverDotFill: string;
  mouseButtonSettings: ButtonSetting[];
};
type Update = { version: string; url: string };
type View = { settings: Settings; update: Update | null; warning: string | null };
const buttonNames = ["左键", "中键", "右键", "后退侧键", "前进侧键"];

function Toggle({ label, checked, change, disabled = false }: {
  label: string; checked: boolean; change: (value: boolean) => void; disabled?: boolean;
}) {
  return <label className="toggle-row">
    <span>{label}</span>
    <input type="checkbox" role="switch" checked={checked} disabled={disabled}
      onChange={e => change(e.target.checked)} />
  </label>;
}

function Size({ label, value, min, max, change }: {
  label: string; value: number; min: number; max: number; change: (value: number) => void;
}) {
  return <label className="size-row">
    <span>{label}</span>
    <input type="range" min={min} max={max} step={5} value={value}
      onChange={e => change(e.target.valueAsNumber)} />
    <output>{value} <small>DIP</small></output>
  </label>;
}

function ColorField({ label, value, change }: {
  label: string; value: string; change: (value: string) => void;
}) {
  const [draft, setDraft] = useState(value);
  const [invalid, setInvalid] = useState(false);
  useEffect(() => { setDraft(value); setInvalid(false); }, [value]);
  function commit() {
    let hex = draft.trim().replace(/^#/, "");
    if (!/^(?:[a-f0-9]{3,4}|[a-f0-9]{6}|[a-f0-9]{8})$/i.test(hex)) {
      setInvalid(true);
      return;
    }
    if (hex.length <= 4) hex = [...hex].map(c => c + c).join("");
    const argb = "#" + (hex.length === 6 ? "FF" + hex : hex).toUpperCase();
    setDraft(argb);
    setInvalid(false);
    if (argb !== value) change(argb);
  }
  return <div className="color-field">
    <input type="color" value={"#" + value.slice(3)} aria-label={label + "色板"}
      onChange={e => change(value.slice(0, 3) + e.target.value.slice(1).toUpperCase())} />
    <input className="hex" value={draft} aria-label={label + " ARGB"} aria-invalid={invalid}
      title="ARGB 十六进制颜色，前两位为透明度" spellCheck={false} maxLength={9}
      onChange={e => { setDraft(e.target.value); setInvalid(false); }} onBlur={commit}
      onKeyDown={e => { if (e.key === "Enter") e.currentTarget.blur(); }} />
    {invalid && <small role="alert">请输入十六进制颜色，如 #60FFFF5A</small>}
  </div>;
}

export default function App() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [startup, setStartup] = useState<boolean | null>(null);
  const [changingStartup, setChangingStartup] = useState(false);
  const [update, setUpdate] = useState<Update | null>(null);
  const [warning, setWarning] = useState("");
  const [error, setError] = useState("");
  const [restoring, setRestoring] = useState(false);
  const latest = useRef<Settings | null>(null);
  const pending = useRef<Settings | null>(null);
  const saving = useRef<Promise<void> | null>(null);

  function display(value: Settings) {
    latest.current = value;
    setSettings(value);
  }

  useEffect(() => {
    let active = true;
    const updates = listen<Update>("update-available", e => { if (active) setUpdate(e.payload); });
    const warnings = listen<string>("settings-warning", e => { if (active) setWarning(e.payload); });
    Promise.all([updates, warnings]).then(() => invoke<View>("preferences")).then(view => {
      if (!active) return;
      display(view.settings);
      setWarning(view.warning ?? "");
      setUpdate(view.update);
    }).catch(e => { if (active) setError(String(e)); });
    invoke<boolean>("autostart_enabled").then(value => { if (active) setStartup(value); })
      .catch(e => { if (active) setError(String(e)); });
    return () => {
      active = false;
      for (const listener of [updates, warnings]) void listener.then(unlisten => unlisten()).catch(() => {});
    };
  }, []);

  function edit(patch: Partial<Settings>) {
    if (!latest.current) return;
    const next = { ...latest.current, ...patch };
    display(next);
    pending.current = next;
    if (saving.current) return;
    // Coalesce slider input while a disk write is running and preserve edit order.
    saving.current = (async () => {
      while (pending.current) {
        const snapshot = pending.current;
        pending.current = null;
        try {
          await invoke("save_preferences", { settings: snapshot });
          setError("");
        } catch (e) {
          setError(String(e));
          if (!pending.current) {
            const view = await invoke<View>("preferences");
            if (!pending.current) display(view.settings);
          }
        }
      }
    })().catch(e => setError(String(e))).finally(() => { saving.current = null; });
  }

  function editButton(index: number, patch: Partial<ButtonSetting>) {
    if (!latest.current) return;
    edit({ mouseButtonSettings: latest.current.mouseButtonSettings.map((b, i) => i === index ? { ...b, ...patch } : b) });
  }

  async function restore() {
    setRestoring(true);
    try {
      await saving.current;
      display(await invoke<Settings>("restore_defaults"));
      setError("");
    } catch (e) { setError(String(e)); }
    finally { setRestoring(false); }
  }

  async function changeStartup(enabled: boolean) {
    setChangingStartup(true);
    try {
      await invoke("set_autostart", { enabled });
      setStartup(enabled);
      setError("");
    } catch (e) { setError(String(e)); }
    finally { setChangingStartup(false); }
  }

  function open(url: string) { void openUrl(url).catch(e => setError(String(e))); }

  return <main>
    <header><img src={icon} width="32" height="32" alt="" /><h1>ClickShow</h1></header>
    {warning && <p className="notice" role="status">{warning}</p>}
    {error && <p className="notice" role="alert">{error}</p>}
    {update && <p className="notice">原项目有新版本 {update.version}　
      <button className="link" onClick={() => open(update.url)}>查看版本</button></p>}
    {settings ? <fieldset disabled={restoring}>
      <section aria-labelledby="effects"><h2 id="effects">鼠标提示</h2>
        <div className="group">
          <Toggle label="点击波纹" checked={settings.enableClickCircle} change={v => edit({ enableClickCircle: v })} />
          <Toggle label="位置浮标" checked={settings.enableHoverDot} change={v => edit({ enableHoverDot: v })} />
        </div>
      </section>
      <section aria-labelledby="appearance"><h2 id="appearance">外观</h2>
        <div className="group">
          <Size label="波纹大小" value={settings.indicatorSize} min={50} max={400} change={v => edit({ indicatorSize: v })} />
          <Size label="浮标大小" value={settings.hoverDotSize} min={10} max={150} change={v => edit({ hoverDotSize: v })} />
          <div className="row"><span>浮标颜色</span>
            <ColorField label="浮标颜色" value={settings.hoverDotFill} change={v => edit({ hoverDotFill: v })} /></div>
          <details><summary>各按键的波纹和颜色</summary>
            <p className="hint">颜色格式为 #AARRGGBB，前两位控制透明度。</p>
            {settings.mouseButtonSettings.map((button, i) => <div className="row button-row" key={i}>
              <label><input type="checkbox" checked={button.isEnabled}
                onChange={e => editButton(i, { isEnabled: e.target.checked })} />{buttonNames[i]}</label>
              <ColorField label={buttonNames[i] + "波纹"} value={button.color} change={v => editButton(i, { color: v })} />
            </div>)}
          </details>
        </div>
        <button className="link reset" onClick={restore}>恢复默认外观</button>
      </section>
      <section aria-labelledby="startup"><h2 id="startup">启动</h2><div className="group">
        <Toggle label="登录时自动启动" checked={startup ?? false} disabled={startup === null || changingStartup} change={changeStartup} />
      </div></section>
    </fieldset> : !error && <p>正在读取设置…</p>}
    <footer><button className="link" onClick={() => open("https://github.com/cuiliang/ClickShow")}>原项目主页 ↗</button>
      <button className="quit" onClick={async () => { await saving.current; await invoke("quit").catch(e => setError(String(e))); }}>退出 ClickShow</button></footer>
  </main>;
}
