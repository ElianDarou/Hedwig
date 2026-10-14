import { FormEvent, type DragEvent, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  AdminGroup, AdminSpace, AdminUser, api, BackupConfiguration, BackupStatus, Change, CloudFile, CloudFolder, Device, downloadDiagnostics, downloadFile, downloadFolderArchive, downloadSharedFile, downloadVersion, fetchFileBlob, fetchVersionText, FileVersion, FolderRestorePreview, MirrorConfiguration, MirrorStatus, SystemStatus,
  ApiError, formatBytes, Session, Share, Space, SpaceAccess, uploadFile, uploadToShare,
} from "./api";

type ViewState = "loading" | "setup" | "login" | "files";

export function App() {
  const publicToken = readShareToken();
  const [view, setView] = useState<ViewState>("loading");
  const [session, setSession] = useState<Session | null>(() => readSession());
  const [spaces, setSpaces] = useState<Space[]>([]);
  const [selectedSpace, setSelectedSpace] = useState<string>("");
  const [files, setFiles] = useState<CloudFile[]>([]);
  const [folders, setFolders] = useState<CloudFolder[]>([]);
  const [currentPath, setCurrentPath] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [section, setSection] = useState<"files" | "activities" | "shares" | "trash" | "settings" | "admin">("files");
  const [shares, setShares] = useState<Share[]>([]);
  const [changes, setChanges] = useState<Change[]>([]);
  const [error, setError] = useState("");

  useEffect(() => {
    if (session) {
      setView("files");
      void loadSpaces(session);
      return;
    }
    api.setupStatus().then(({ setup_required }) => setView(setup_required ? "setup" : "login"))
      .catch((reason) => { setError(reason.message); setView("login"); });
  }, [session]);

  async function loadSpaces(activeSession: Session) {
    try {
      const result = await api.spaces(activeSession.token);
      setSpaces(result.spaces);
      setSelectedSpace((current) => result.spaces.some((space) => space.id === current)
        ? current : activeSession.personal_space_id || result.spaces[0]?.id || "");
    } catch (reason) {
      setError(messageOf(reason));
    }
  }

  useEffect(() => {
    if (!session || !selectedSpace) return;
    void refreshContent(session, selectedSpace, section === "trash", setFiles, setFolders, setError);
  }, [session, selectedSpace, section]);

  useEffect(() => {
    if (!session || section !== "shares") return;
    api.shares(session.token).then((result) => setShares(result.shares)).catch((reason) => setError(messageOf(reason)));
  }, [session, section]);

  useEffect(() => {
    if (!session || section !== "activities") return;
    api.changes(session.token, 0, 500).then((result) => setChanges(result.changes.reverse()))
      .catch((reason) => setError(messageOf(reason)));
  }, [session, section]);

  function onAuthenticated(next: Session) {
    sessionStorage.setItem("cloud-session", JSON.stringify(next));
    setSession(next);
    setError("");
  }

  async function logout() {
    if (session) {
      try {
        await api.logout(session.token);
      } catch (reason) {
        if (!(reason instanceof ApiError && reason.status === 401)) {
          setError(`Abmelden am Server fehlgeschlagen: ${messageOf(reason)}`);
          return;
        }
      }
    }
    sessionStorage.removeItem("cloud-session");
    setSession(null);
    setSpaces([]);
    setFiles([]);
    setView("login");
  }

  async function createSharedSpace() {
    const name = window.prompt("Name des gemeinsamen Speicherbereichs");
    if (!name || !session) return;
    try {
      const created = await api.createSpace(session.token, name);
      await loadSpaces(session); setSelectedSpace(created.id); setCurrentPath(""); setSection("files");
    } catch (reason) { setError(messageOf(reason)); }
  }

  if (publicToken) return <PublicShareScreen token={publicToken} />;
  if (view === "loading") return <main className="centered"><p>Hedwig wird geladen …</p></main>;
  if (view === "setup" || view === "login") {
    return <AuthScreen setup={view === "setup"} error={error} onAuthenticated={onAuthenticated} setError={setError} />;
  }

  const current = spaces.find((space) => space.id === selectedSpace);
  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="product-name">Hedwig</div>
        <nav aria-label="Bereiche">
          <button className={`nav-item ${section === "files" ? "active" : ""}`} onClick={() => setSection("files")}>Dateien</button>
          <button className={`nav-item ${section === "activities" ? "active" : ""}`} onClick={() => setSection("activities")}>Aktivitäten</button>
          <button className={`nav-item ${section === "shares" ? "active" : ""}`} onClick={() => setSection("shares")}>Geteilt</button>
          <button className={`nav-item ${section === "trash" ? "active" : ""}`} onClick={() => setSection("trash")}>Papierkorb</button>
          <button className={`nav-item ${section === "settings" ? "active" : ""}`} onClick={() => setSection("settings")}>Einstellungen</button>
          {session?.is_admin && <button className={`nav-item ${section === "admin" ? "active" : ""}`} onClick={() => setSection("admin")}>Administration</button>}
        </nav>
        <div className="spaces">
          <div className="section-label section-label-row"><span>Speicherbereiche</span><button aria-label="Gemeinsamen Speicherbereich anlegen" onClick={() => void createSharedSpace()}>+</button></div>
          {spaces.map((space) => (
            <button key={space.id} className={`space-item ${space.id === selectedSpace ? "selected" : ""}`}
              onClick={() => { setSelectedSpace(space.id); setCurrentPath(""); }}>
              <span>{space.name}</span><span className="role">{roleLabel(space.role)}</span>
            </button>
          ))}
        </div>
        <button className="account" onClick={() => void logout()}>{session?.username}<span>Abmelden</span></button>
      </aside>
      <main className="content">
        <header className="toolbar">
          <div>
            <h1>{section === "trash" ? "Papierkorb" : section === "activities" ? "Letzte Aktivitäten" : section === "shares" ? "Freigaben" : section === "settings" ? "Sicherheit und Geräte" : section === "admin" ? "Serververwaltung" : current?.name ?? "Dateien"}</h1>
            <div className="breadcrumbs"><button onClick={() => setCurrentPath("")}>/</button>
              {currentPath.split("/").filter(Boolean).map((part, index, parts) => <button key={`${part}-${index}`}
                onClick={() => setCurrentPath(parts.slice(0,index+1).join("/"))}>{part}</button>)}</div>
          </div>
          {section === "files" && <label className="search-field"><span className="visually-hidden">Dateien suchen</span>
            <input type="search" placeholder="Name oder Pfad suchen" value={searchQuery}
              onChange={(event) => setSearchQuery(event.target.value)} /></label>}
          {section === "files" && session && selectedSpace && <UploadButton token={session.token} spaceId={selectedSpace} path={currentPath}
            onDone={() => refreshContent(session, selectedSpace, false, setFiles, setFolders, setError)} />}
        </header>
        {error && <div className="error" role="alert">{error}<button onClick={() => setError("")} aria-label="Meldung schließen">×</button></div>}
        {section === "admin" ? <AdminView session={session!} setError={setError} /> : section === "settings" ? <SecuritySettings session={session!} space={current} setError={setError} onSpaceChanged={() => loadSpaces(session!)} /> : section === "activities" ? <ActivitiesView changes={changes} spaces={spaces} /> : section === "shares" ? <SharesView shares={shares} spaces={spaces} token={session!.token} onChanged={() => {
          void api.shares(session!.token).then((result) => setShares(result.shares)).catch((reason) => setError(messageOf(reason)));
        }} setError={setError} /> : section === "files" && searchQuery.trim() ? <SearchResults
          files={files.filter((file) => !file.deleted_at && file.path.toLocaleLowerCase("de").includes(searchQuery.trim().toLocaleLowerCase("de")))}
          token={session!.token} spaceId={selectedSpace} spaces={spaces} setError={setError}
          onChanged={() => refreshContent(session!, selectedSpace, false, setFiles, setFolders, setError)} />
          : section === "files" ? <FolderBrowser files={files.filter((file) => !file.deleted_at)} folders={folders} path={currentPath}
          onOpenFolder={setCurrentPath} token={session!.token} spaceId={selectedSpace} spaces={spaces} setError={setError}
          onChanged={() => refreshContent(session!, selectedSpace, false, setFiles, setFolders, setError)} /> : <FileTable files={files.filter((file) => file.deleted_at)}
          token={session!.token} spaceId={selectedSpace} spaces={spaces} trash={section === "trash"} setError={setError}
          onChanged={() => refreshContent(session!, selectedSpace, section === "trash", setFiles, setFolders, setError)} />}
      </main>
    </div>
  );
}

function ActivitiesView({ changes, spaces }: { changes: Change[]; spaces: Space[] }) {
  if (!changes.length) return <div className="empty"><strong>Noch keine Aktivitäten</strong><span>Bestätigte Änderungen erscheinen hier.</span></div>;
  return <div className="activity-list">{changes.map((change) => <article key={change.event_id}>
    <span className="activity-mark" aria-hidden>{activityIcon(change.kind)}</span>
    <div><strong>{activityLabel(change.kind)}</strong><p>{change.path}</p></div>
    <span>{spaces.find((space) => space.id === change.space_id)?.name ?? "Speicherbereich"}</span>
    <time dateTime={change.created_at}>{new Date(change.created_at).toLocaleString("de-DE")}</time>
  </article>)}</div>;
}

function activityLabel(kind: string): string {
  return ({ content_changed: "Datei geändert", created: "Datei erstellt", moved: "Datei verschoben", deleted: "Datei gelöscht",
    restored: "Datei wiederhergestellt", conflict_created: "Konfliktkopie erstellt", folder_created: "Ordner erstellt",
    folder_deleted: "Ordner gelöscht", share_created: "Freigabe erstellt", share_revoked: "Freigabe widerrufen" } as Record<string,string>)[kind] ?? kind;
}
function activityIcon(kind: string): string { return kind.includes("deleted") || kind.includes("revoked") ? "−" : kind === "moved" ? "→" : "+"; }

function AuthScreen({ setup, error, onAuthenticated, setError }: {
  setup: boolean; error: string; onAuthenticated: (session: Session) => void; setError: (value: string) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [recovering, setRecovering] = useState(false);
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); setBusy(true); setError("");
    const data = new FormData(event.currentTarget);
    try {
      const username = String(data.get("username"));
      if (recovering) onAuthenticated(await api.recover(username, String(data.get("recovery_code"))));
      else {
        const password = String(data.get("password"));
        onAuthenticated(setup ? await api.setup(username, password) : await api.login(username, password, String(data.get("otp") || "")));
      }
    } catch (reason) { setError(messageOf(reason)); } finally { setBusy(false); }
  }
  return (
    <main className="auth-page">
      <form className="auth-form" onSubmit={submit}>
        <div className="product-name">Hedwig</div>
        <h1>{setup ? "Server einrichten" : recovering ? "Konto wiederherstellen" : "Anmelden"}</h1>
        <p>{setup ? "Lege das erste Administratorkonto an. Es gibt kein Standardpasswort." : recovering ? "Ein Wiederherstellungscode wird nach dieser Anmeldung ungültig." : "Melde dich an deinem Cloud-Server an."}</p>
        {error && <div className="error" role="alert">{error}</div>}
        <label htmlFor="username">Benutzername</label>
        <input id="username" name="username" autoComplete="username" required minLength={3} autoFocus />
        {recovering ? <><label htmlFor="recovery-code">Wiederherstellungscode</label>
          <input id="recovery-code" name="recovery_code" autoComplete="off" required /></> : <>
          <label htmlFor="password">Passwort</label>
          <input id="password" name="password" type="password" autoComplete={setup ? "new-password" : "current-password"}
            required minLength={setup ? 12 : undefined} />
        {!setup && <><label htmlFor="otp">2FA-Code (falls aktiviert)</label>
          <input id="otp" name="otp" inputMode="numeric" pattern="[0-9]{6}" autoComplete="one-time-code" /></>}
        </>}
        <button className="primary" disabled={busy}>{busy ? "Bitte warten …" : setup ? "Server einrichten" : recovering ? "Code verwenden" : "Anmelden"}</button>
        {!setup && <button type="button" className="text-button" onClick={() => setRecovering((value) => !value)}>
          {recovering ? "Zurück zur Anmeldung" : "Mit Wiederherstellungscode anmelden"}
        </button>}
      </form>
    </main>
  );
}

function UploadButton({ token, spaceId, path, onDone }: { token: string; spaceId: string; path: string; onDone: () => void }) {
  const input = useRef<HTMLInputElement>(null);
  const [progress, setProgress] = useState<{ value: number; text: string } | null>(null);
  const [error, setError] = useState("");
  async function selected(files: File[]) {
    if (!files.length) return;
    setError(""); setProgress({ value: 0, text: "Upload wird vorbereitet …" });
    try {
      for (const [index,file] of files.entries()) {
        const relative = file.webkitRelativePath || file.name;
        const target = [path,relative].filter(Boolean).join("/");
        await uploadFile(token, spaceId, file, (value, text) => setProgress({
          value: ((index + value / 100) / files.length) * 100,
          text: files.length > 1 ? `${index + 1}/${files.length}: ${text}` : text,
        }), target);
      }
      onDone();
      window.setTimeout(() => setProgress(null), 1200);
    } catch (reason) { setError(messageOf(reason)); setProgress(null); }
    if (input.current) input.current.value = "";
  }
  return <div className="upload-control">
    <input ref={input} className="visually-hidden" type="file" multiple onChange={(event) => void selected(Array.from(event.target.files ?? []))} />
    <button className="primary" onClick={() => input.current?.click()}>Dateien hochladen</button>
    {progress && <div className="upload-progress" role="status"><progress value={progress.value} max="100" />{progress.text}</div>}
    {error && <div className="upload-error" role="alert">{error}</div>}
  </div>;
}

function FolderBrowser({ files, folders, path, onOpenFolder, token, spaceId, spaces, setError, onChanged }: {
  files: CloudFile[]; folders: CloudFolder[]; path: string; onOpenFolder: (path: string) => void; token: string; spaceId: string;
  spaces: Space[]; setError: (message: string) => void; onChanged: () => void;
}) {
  const [sharingFolder, setSharingFolder] = useState<string | null>(null);
  const [restoringFolder, setRestoringFolder] = useState<string | null>(null);
  const prefix = path ? `${path}/` : "";
  const children = files.filter((file) => file.path.startsWith(prefix));
  const explicit = folders.filter((folder) => folder.path.startsWith(prefix));
  const inferredNames = children.map((file) => file.path.slice(prefix.length)).filter((relative) => relative.includes("/"))
    .map((relative) => relative.split("/")[0]);
  const explicitNames = explicit.map((folder) => folder.path.slice(prefix.length)).filter((relative) => relative && !relative.includes("/"));
  const folderNames = [...new Set([...inferredNames, ...explicitNames])].sort((left,right) => left.localeCompare(right,"de"));
  const directFiles = children.filter((file) => !file.path.slice(prefix.length).includes("/"));
  async function createFolder() {
    const name = window.prompt("Name des neuen Ordners");
    if (!name) return;
    try {
      await api.createFolder(token, spaceId, [path,name].filter(Boolean).join("/"));
      onChanged();
    } catch (reason) { setError(messageOf(reason)); }
  }
  async function removeFolder(folderName: string) {
    const fullPath = [path,folderName].filter(Boolean).join("/");
    const record = folders.find((folder) => folder.path === fullPath);
    if (!record) {
      setError("Dieser aus Dateipfaden abgeleitete Ordner kann erst gelöscht werden, nachdem er als Ordner erfasst wurde.");
      return;
    }
    if (!window.confirm(`Ordner „${folderName}“ und alle enthaltenen Dateien in den Papierkorb verschieben?`)) return;
    try { await api.deleteFolder(token, record.id); onChanged(); }
    catch (reason) { setError(messageOf(reason)); }
  }
  async function renameFolder(folderName: string) {
    const current = [path,folderName].filter(Boolean).join("/");
    const record = folders.find((folder) => folder.path === current);
    if (!record) {
      setError("Dieser aus Dateipfaden abgeleitete Ordner ist noch nicht als Ordner erfasst.");
      return;
    }
    const target = window.prompt("Neuer relativer Ordnerpfad", current);
    if (!target || target === current) return;
    try { await api.moveFolder(token, record.id, target); onChanged(); }
    catch (reason) { setError(messageOf(reason)); }
  }
  function dropped(event: DragEvent<HTMLDivElement>) {
    event.preventDefault();
    const droppedFiles = Array.from(event.dataTransfer.files);
    void (async () => {
      try {
        for (const file of droppedFiles) {
          await uploadFile(token, spaceId, file, () => undefined, [path,file.name].filter(Boolean).join("/"));
        }
        onChanged();
      } catch (reason) { setError(messageOf(reason)); }
    })();
  }
  return <div className="folder-browser" onDragOver={(event) => event.preventDefault()} onDrop={dropped}>
    <div className="folder-tools"><button className="secondary" onClick={() => setRestoringFolder(path)}>Früheren Stand</button>
      <button className="secondary" onClick={() => void downloadFolderArchive(token, spaceId, path)
      .catch((reason) => setError(messageOf(reason)))}>Als ZIP laden</button>
      <button className="secondary" onClick={() => void createFolder()}>Neuer Ordner</button></div>
    {folderNames.length > 0 && <div className="folder-grid">{folderNames.map((folder) => <div className="folder-card" key={folder}>
      <button className="folder-open" onClick={() => onOpenFolder([path,folder].filter(Boolean).join("/"))}><span aria-hidden>▰</span>{folder}</button>
      <button className="folder-rename" aria-label={`Ordner ${folder} umbenennen oder verschieben`} onClick={() => void renameFolder(folder)}>✎</button>
      <button className="folder-history" aria-label={`Früheren Stand von ${folder} wiederherstellen`} onClick={() => setRestoringFolder([path,folder].filter(Boolean).join("/"))}>↶</button>
      <button className="folder-share" aria-label={`Ordner ${folder} teilen`} onClick={() => setSharingFolder([path,folder].filter(Boolean).join("/"))}>↗</button>
      <button className="folder-download" aria-label={`Ordner ${folder} als ZIP laden`} onClick={() => void downloadFolderArchive(token, spaceId, [path,folder].filter(Boolean).join("/"))
        .catch((reason) => setError(messageOf(reason)))}>↓</button>
      <button className="folder-delete" aria-label={`Ordner ${folder} löschen`} onClick={() => void removeFolder(folder)}>×</button>
    </div>)}</div>}
    {directFiles.length > 0 ? <FileTable files={directFiles} token={token} spaceId={spaceId} spaces={spaces} trash={false} setError={setError} onChanged={onChanged} />
      : folderNames.length === 0 && <div className="empty"><strong>Dieser Ordner ist leer</strong><span>Lege einen Ordner an oder lade Dateien hoch.</span></div>}
    <p className="drop-hint">Dateien können auch hierher gezogen werden.</p>
    {sharingFolder && <PathShareDialog token={token} spaceId={spaceId} path={sharingFolder} mode="read"
      onClose={() => setSharingFolder(null)} onError={setError} />}
    {restoringFolder !== null && <FolderRestoreDialog token={token} spaceId={spaceId} path={restoringFolder}
      onClose={() => setRestoringFolder(null)} onError={setError} onRestored={onChanged} />}
  </div>;
}

function FolderRestoreDialog({ token, spaceId, path, onClose, onError, onRestored }: {
  token: string; spaceId: string; path: string; onClose: () => void; onError: (message: string) => void; onRestored: () => void;
}) {
  const initial = new Date(Date.now() - 60 * 60 * 1000);
  const [at, setAt] = useState(toLocalDateTime(initial));
  const [preview, setPreview] = useState<FolderRestorePreview | null>(null);
  const [busy, setBusy] = useState(false);
  async function inspect(event: FormEvent) {
    event.preventDefault(); setBusy(true);
    try { setPreview(await api.folderRestorePreview(token, spaceId, path, new Date(at).toISOString())); }
    catch (reason) { onError(messageOf(reason)); } finally { setBusy(false); }
  }
  async function restore() {
    if (!preview || preview.unavailable.length || !window.confirm("Den angezeigten Ordnerstand als neue aktuelle Fassung wiederherstellen? Neuere Versionen bleiben in der Historie.")) return;
    setBusy(true);
    try { await api.restoreFolder(token, spaceId, path, preview.target_at); onRestored(); onClose(); }
    catch (reason) { onError(messageOf(reason)); } finally { setBusy(false); }
  }
  return <div className="modal-backdrop" role="presentation"><section className="modal restore-modal" role="dialog" aria-modal="true" aria-labelledby="restore-folder-title">
    <h2 id="restore-folder-title">Ordnerstand wiederherstellen</h2><p className="muted">{path || "Gesamter Speicherbereich"}</p>
    <form onSubmit={(event) => void inspect(event)}><label>Zeitpunkt<input type="datetime-local" value={at} max={toLocalDateTime(new Date())} onChange={(event) => { setAt(event.target.value); setPreview(null); }} required /></label>
      <button className="secondary" disabled={busy}>{busy ? "Prüfen …" : "Auswirkungen prüfen"}</button></form>
    {preview && <div className="restore-preview"><strong>{preview.actions.length} geplante Änderung(en)</strong>
      {preview.actions.length === 0 && <p>Der Ordner entspricht bereits diesem Stand.</p>}
      <ul>{preview.actions.map((action) => <li key={`${action.file_id}-${action.action}`}><span>{restoreActionLabel(action.action)}</span>{action.path}</li>)}</ul>
      {preview.unavailable.length > 0 && <div className="restore-missing"><strong>Nicht wiederherstellbar</strong><ul>{preview.unavailable.map((item) => <li key={item}>{item}</li>)}</ul></div>}</div>}
    <div className="dialog-actions"><button className="secondary" onClick={onClose}>Abbrechen</button>
      <button className="primary" disabled={busy || !preview || preview.actions.length === 0 || preview.unavailable.length > 0} onClick={() => void restore()}>Wiederherstellen</button></div>
  </section></div>;
}

function restoreActionLabel(action: FolderRestorePreview["actions"][number]["action"]): string {
  return ({ delete: "Entfernen", restore: "Zurückholen", move: "Zurückverschieben", restore_version: "Version zurücksetzen" } as const)[action];
}
function toLocalDateTime(date: Date): string {
  return new Date(date.getTime() - date.getTimezoneOffset() * 60_000).toISOString().slice(0,16);
}

function SearchResults({ files, token, spaceId, spaces, setError, onChanged }: {
  files: CloudFile[]; token: string; spaceId: string; spaces: Space[]; setError: (message: string) => void; onChanged: () => void;
}) {
  return <section className="search-results"><p>{files.length} Treffer</p>
    <FileTable files={files} token={token} spaceId={spaceId} spaces={spaces} trash={false} setError={setError} onChanged={onChanged} /></section>;
}

function FileTable({ files, token, spaceId, spaces, trash, setError, onChanged }: {
  files: CloudFile[]; token: string; spaceId: string; spaces: Space[]; trash: boolean; setError: (message: string) => void; onChanged: () => void;
}) {
  const [openVersions, setOpenVersions] = useState<string | null>(null);
  const [versions, setVersions] = useState<FileVersion[]>([]);
  const [sharing, setSharing] = useState<CloudFile | null>(null);
  const [previewing, setPreviewing] = useState<CloudFile | null>(null);
  const [copying, setCopying] = useState<CloudFile | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  useEffect(() => {
    setSelected((current) => new Set([...current].filter((id) => files.some((file) => file.id === id))));
  }, [files]);
  async function showVersions(fileId: string) {
    if (openVersions === fileId) { setOpenVersions(null); return; }
    try { setVersions((await api.versions(token, fileId)).versions); setOpenVersions(fileId); }
    catch (reason) { setError(messageOf(reason)); }
  }
  async function rename(file: CloudFile) {
    const next = window.prompt("Neuer relativer Pfad", file.path);
    if (!next || next === file.path) return;
    try { await api.move(token, file.id, next); onChanged(); }
    catch (reason) { setError(messageOf(reason)); }
  }
  async function mutate(action: () => Promise<unknown>) {
    try { await action(); onChanged(); } catch (reason) { setError(messageOf(reason)); }
  }
  async function bulk(action: "download" | "delete" | "restore") {
    const targets = files.filter((file) => selected.has(file.id));
    if (!targets.length) return;
    if (action === "delete" && !window.confirm(`${targets.length} Datei(en) in den Papierkorb verschieben?`)) return;
    try {
      if (action === "download") {
        for (const file of targets) await downloadFile(token, file);
      } else {
        await Promise.all(targets.map((file) => action === "delete" ? api.remove(token, file.id) : api.restore(token, file.id)));
      }
      setSelected(new Set()); onChanged();
    } catch (reason) { setError(messageOf(reason)); }
  }
  if (files.length === 0) return <div className="empty"><strong>{trash ? "Papierkorb ist leer" : "Noch keine Dateien"}</strong>
    <span>{trash ? "Gelöschte Dateien erscheinen hier." : "Lade eine Datei hoch, um diesen Bereich zu füllen."}</span></div>;
  const allSelected = files.length > 0 && files.every((file) => selected.has(file.id));
  return <>{selected.size > 0 && <div className="bulk-actions"><strong>{selected.size} ausgewählt</strong>
    {!trash && <button className="secondary" onClick={() => void bulk("download")}>Herunterladen</button>}
    <button className={`secondary ${trash ? "" : "danger-button"}`} onClick={() => void bulk(trash ? "restore" : "delete")}>{trash ? "Wiederherstellen" : "Löschen"}</button>
    <button className="text-button" onClick={() => setSelected(new Set())}>Auswahl aufheben</button></div>}
  <div className="table-wrap"><table>
    <thead><tr><th><label className="select-file"><input type="checkbox" checked={allSelected}
      onChange={(event) => setSelected(event.target.checked ? new Set(files.map((file) => file.id)) : new Set())} />
      <span>Name</span></label></th><th>Geändert</th><th>Größe</th><th><span className="visually-hidden">Aktionen</span></th></tr></thead>
    <tbody>{files.map((file) => <FileRows key={file.id} file={file} token={token} trash={trash}
      selected={selected.has(file.id)} onSelected={(checked) => setSelected((current) => {
        const next = new Set(current); if (checked) next.add(file.id); else next.delete(file.id); return next;
      })}
      expanded={openVersions === file.id} versions={versions} onVersions={() => void showVersions(file.id)}
      onRename={() => void rename(file)} onError={setError}
      onCopy={() => setCopying(file)}
      onPreview={() => setPreviewing(file)}
      onShare={() => setSharing(file)}
      onDelete={() => void mutate(() => api.remove(token, file.id))}
      onRestore={() => void mutate(() => api.restore(token, file.id))} />)}</tbody>
  </table></div>{sharing && <ShareDialog token={token} spaceId={spaceId} file={sharing}
    onClose={() => setSharing(null)} onError={setError} />}
    {previewing && <PreviewDialog token={token} file={previewing} onClose={() => setPreviewing(null)} onError={setError} />}
    {copying && <CopyDialog token={token} file={copying} sourceSpaceId={spaceId} spaces={spaces} onClose={() => setCopying(null)} onError={setError} onCopied={() => { setCopying(null); onChanged(); }} />}</>;
}

function CopyDialog({ token, file, sourceSpaceId, spaces, onClose, onError, onCopied }: { token: string; file: CloudFile; sourceSpaceId: string; spaces: Space[]; onClose: () => void; onError: (message: string) => void; onCopied: () => void }) {
  const directory = file.path.includes("/") ? file.path.slice(0,file.path.lastIndexOf("/") + 1) : "";
  const name = file.path.split("/").at(-1) ?? file.path; const dot = name.lastIndexOf(".");
  const suggested = dot > 0 ? `${directory}${name.slice(0,dot)} (Kopie)${name.slice(dot)}` : `${directory}${name} (Kopie)`;
  const targets = spaces.filter((space) => ["owner","manager","editor","uploader"].includes(space.role));
  const [targetSpace,setTargetSpace] = useState(sourceSpaceId); const [path,setPath] = useState(suggested); const [busy,setBusy] = useState(false);
  async function submit(event: FormEvent<HTMLFormElement>) { event.preventDefault(); setBusy(true); try { await api.copy(token,file.id,path,targetSpace); onCopied(); } catch (reason) { onError(messageOf(reason)); } finally { setBusy(false); } }
  return <div className="modal-backdrop" role="presentation" onMouseDown={(event) => { if (event.currentTarget === event.target) onClose(); }}><form className="modal" role="dialog" aria-modal="true" aria-labelledby="copy-title" onSubmit={(event) => void submit(event)}><h2 id="copy-title">Datei kopieren</h2><label>Zielbereich<select value={targetSpace} onChange={(event) => setTargetSpace(event.target.value)}>{targets.map((space) => <option key={space.id} value={space.id}>{space.name}</option>)}</select></label><label>Relativer Zielpfad<input value={path} onChange={(event) => setPath(event.target.value)} required /></label>{targetSpace !== sourceSpaceId && <p className="muted">Beim Kopieren in einen anderen Bereich werden die Inhaltsblöcke dort getrennt gespeichert und gegen die dortige Quote geprüft.</p>}<div className="dialog-actions"><button type="button" className="secondary" onClick={onClose}>Abbrechen</button><button className="primary" disabled={busy || !path.trim()}>{busy ? "Wird kopiert …" : "Kopieren"}</button></div></form></div>;
}

function FileRows({ file, token, trash, selected, onSelected, expanded, versions, onVersions, onRename, onCopy, onPreview, onShare, onDelete, onRestore, onError }: {
  file: CloudFile; token: string; trash: boolean; expanded: boolean; versions: FileVersion[];
  selected: boolean; onSelected: (checked: boolean) => void;
  onVersions: () => void; onRename: () => void; onCopy: () => void; onPreview: () => void; onShare: () => void; onDelete: () => void; onRestore: () => void; onError: (message: string) => void;
}) {
  const [comparing, setComparing] = useState<FileVersion | null>(null);
  async function versionAction(action: () => Promise<unknown>) {
    try { await action(); onVersions(); window.setTimeout(onVersions, 0); }
    catch (reason) { onError(messageOf(reason)); }
  }
  return <>
    <tr>
      <td><label className="select-file"><input type="checkbox" checked={selected} onChange={(event) => onSelected(event.target.checked)} />
        <span><span className="file-icon" aria-hidden>▧</span>{file.path}</span></label></td>
      <td>{new Intl.DateTimeFormat("de-DE", { dateStyle: "medium", timeStyle: "short" }).format(new Date(file.updated_at))}</td>
      <td>{formatBytes(file.size)}</td>
      <td><div className="row-actions">{trash ? <button className="text-button" onClick={onRestore}>Wiederherstellen</button> : <>
        {canPreview(file.path) && <button className="text-button" onClick={onPreview}>Vorschau</button>}
        <button className="text-button" onClick={() => void downloadFile(token, file).catch((reason) => onError(messageOf(reason)))}>Download</button>
        <button className="text-button" onClick={onVersions}>Versionen</button>
        <button className="text-button" onClick={onShare}>Teilen</button>
        <button className="text-button" onClick={onCopy}>Kopieren</button>
        <button className="text-button" onClick={onRename}>Umbenennen</button>
        <button className="text-button danger-button" onClick={onDelete}>Löschen</button>
      </>}</div></td>
    </tr>
    {expanded && <tr className="version-row"><td colSpan={4}>
      <div className="versions"><strong>Versionshistorie</strong>{versions.map((version) => <div key={version.id}>
        <span>{new Intl.DateTimeFormat("de-DE", { dateStyle: "medium", timeStyle: "short" }).format(new Date(version.created_at))}</span>
        <span>{version.created_by} · {version.device_name}</span><span>{formatBytes(version.size)}</span>
        <span className="version-actions">{version.current ? <span>Aktuell</span> : <>
          <button className="text-button" onClick={() => void downloadVersion(token, file, version.id).catch((reason) => onError(messageOf(reason)))}>Download</button>
          {previewKind(file.path) === "text" && version.size <= 1024 * 1024 && file.size <= 1024 * 1024 && <button className="text-button" onClick={() => setComparing(version)}>Vergleichen</button>}
          <button className="text-button" onClick={() => void versionAction(() => api.restoreVersion(token, file.id, version.id))}>Wiederherstellen</button>
        </>}
        <button className="text-button" onClick={() => {
          const name = window.prompt("Name der angehefteten Version", version.pinned_name ?? "");
          if (name !== null) void versionAction(() => api.pinVersion(token, file.id, version.id, name || null));
        }}>{version.pinned_name ? "Angeheftet" : "Anheften"}</button></span>
      </div>)}</div>
    </td></tr>}
    {comparing && createPortal(<VersionDiffDialog token={token} file={file} version={comparing} onClose={() => setComparing(null)} onError={onError} />, document.body)}
  </>;
}

type DiffLine = { kind: "same" | "added" | "removed"; text: string; oldLine: number | null; newLine: number | null };
function VersionDiffDialog({ token, file, version, onClose, onError }: { token: string; file: CloudFile; version: FileVersion; onClose: () => void; onError: (message: string) => void }) {
  const [lines, setLines] = useState<DiffLine[] | null>(null);
  useEffect(() => {
    let active = true;
    Promise.all([fetchVersionText(token, file.id, version.id), fetchFileBlob(token, file).then((blob) => blob.text())])
      .then(([oldText,newText]) => { if (active) setLines(lineDiff(oldText,newText)); })
      .catch((reason) => onError(messageOf(reason)));
    return () => { active = false; };
  }, [token,file.id,version.id]);
  return <div className="modal-backdrop" role="presentation" onMouseDown={(event) => { if (event.currentTarget === event.target) onClose(); }}><section className="modal diff-modal" role="dialog" aria-modal="true" aria-labelledby="diff-title"><header><div><h2 id="diff-title">Zeilenvergleich</h2><p className="muted">Version vom {new Intl.DateTimeFormat("de-DE", { dateStyle: "medium", timeStyle: "short" }).format(new Date(version.created_at))} → aktuelle Fassung</p></div><button className="text-button" onClick={onClose}>Schließen</button></header>{lines ? <div className="line-diff" role="table" aria-label="Textänderungen">{lines.map((line,index) => <div key={`${index}-${line.kind}`} className={`diff-${line.kind}`} role="row"><span>{line.oldLine ?? ""}</span><span>{line.newLine ?? ""}</span><code>{line.kind === "added" ? "+" : line.kind === "removed" ? "−" : " "} {line.text || " "}</code></div>)}</div> : <p>Versionen werden geladen und verglichen …</p>}</section></div>;
}

function lineDiff(oldText: string, newText: string): DiffLine[] {
  const oldLines = oldText.replace(/\r\n/g,"\n").split("\n");
  const newLines = newText.replace(/\r\n/g,"\n").split("\n");
  if (oldLines.length * newLines.length > 1_000_000) {
    return [
      ...oldLines.map((text,index) => ({ kind: "removed" as const, text, oldLine: index + 1, newLine: null })),
      ...newLines.map((text,index) => ({ kind: "added" as const, text, oldLine: null, newLine: index + 1 })),
    ];
  }
  const table = Array.from({ length: oldLines.length + 1 }, () => new Uint32Array(newLines.length + 1));
  for (let left = oldLines.length - 1; left >= 0; left--) for (let right = newLines.length - 1; right >= 0; right--) {
    table[left][right] = oldLines[left] === newLines[right] ? table[left + 1][right + 1] + 1 : Math.max(table[left + 1][right], table[left][right + 1]);
  }
  const result: DiffLine[] = []; let left = 0; let right = 0;
  while (left < oldLines.length || right < newLines.length) {
    if (left < oldLines.length && right < newLines.length && oldLines[left] === newLines[right]) {
      result.push({ kind: "same", text: oldLines[left], oldLine: ++left, newLine: ++right });
    } else if (right < newLines.length && (left === oldLines.length || table[left][right + 1] >= table[left + 1][right])) {
      result.push({ kind: "added", text: newLines[right], oldLine: null, newLine: ++right });
    } else {
      result.push({ kind: "removed", text: oldLines[left], oldLine: ++left, newLine: null });
    }
  }
  return result;
}

function PreviewDialog({ token, file, onClose, onError }: {
  token: string; file: CloudFile; onClose: () => void; onError: (message: string) => void;
}) {
  const [url, setUrl] = useState("");
  const [text, setText] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const kind = previewKind(file.path);
  useEffect(() => {
    let active = true;
    let objectUrl = "";
    void fetchFileBlob(token, file).then(async (blob) => {
      if (!active) return;
      if (kind === "text") setText(await blob.text());
      else { objectUrl = URL.createObjectURL(blob); setUrl(objectUrl); }
    }).catch((reason) => { if (active) { onError(messageOf(reason)); onClose(); } })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; if (objectUrl) URL.revokeObjectURL(objectUrl); };
  }, [token, file.id, kind]);
  return <div className="modal-backdrop preview-backdrop" role="presentation"><section className="modal preview-modal" role="dialog" aria-modal="true" aria-labelledby="preview-title">
    <header><div><h2 id="preview-title">{file.path.split("/").at(-1)}</h2><p className="muted">{formatBytes(file.size)}</p></div>
      <button className="secondary" onClick={onClose}>Schließen</button></header>
    <div className="preview-content">{loading ? <p>Vorschau wird geladen …</p> : kind === "image" ? <img src={url} alt="" />
      : kind === "pdf" ? <iframe src={url} title={`PDF-Vorschau ${file.path}`} sandbox="allow-same-origin" />
      : kind === "audio" ? <audio src={url} controls /> : kind === "video" ? <video src={url} controls />
      : <pre>{text}</pre>}</div>
  </section></div>;
}

function previewKind(path: string): "image" | "pdf" | "text" | "audio" | "video" | null {
  const extension = path.split(".").at(-1)?.toLocaleLowerCase("de") ?? "";
  if (["png","jpg","jpeg","gif","webp","avif","bmp"].includes(extension)) return "image";
  if (extension === "pdf") return "pdf";
  if (["txt","md","markdown","json","csv","xml","yaml","yml","toml","ini","log","rs","go","kt","java","js","jsx","ts","tsx","css"].includes(extension)) return "text";
  if (["mp3","wav","ogg","m4a","aac","flac"].includes(extension)) return "audio";
  if (["mp4","webm","ogv","mov","m4v"].includes(extension)) return "video";
  return null;
}
function canPreview(path: string): boolean { return previewKind(path) !== null; }

function ShareDialog({ token, spaceId, file, onClose, onError }: {
  token: string; spaceId: string; file: CloudFile; onClose: () => void; onError: (message: string) => void;
}) {
  const [password, setPassword] = useState("");
  const [expires, setExpires] = useState("");
  const [createdUrl, setCreatedUrl] = useState("");
  const [busy, setBusy] = useState(false);
  async function create(event: FormEvent) {
    event.preventDefault(); setBusy(true);
    try {
      const result = await api.createShare(token, {
        space_id: spaceId, target_path: file.path, mode: "read",
        ...(password ? { password } : {}),
        ...(expires ? { expires_at: new Date(expires).toISOString() } : {}),
      });
      const url = `${location.origin}${location.pathname}${result.url_fragment}`;
      setCreatedUrl(url);
      await navigator.clipboard?.writeText(url).catch(() => undefined);
    } catch (reason) { onError(messageOf(reason)); } finally { setBusy(false); }
  }
  return <div className="modal-backdrop" role="presentation"><section className="modal" role="dialog" aria-modal="true" aria-labelledby="share-title">
    <h2 id="share-title">Link erstellen</h2>
    <p className="muted">{file.path}</p>
    {createdUrl ? <>
      <label htmlFor="created-link">Freigabelink</label>
      <input id="created-link" value={createdUrl} readOnly onFocus={(event) => event.currentTarget.select()} />
      <p className="muted">Der Link wurde, soweit vom Browser erlaubt, in die Zwischenablage kopiert. Er wird serverseitig nur gehasht gespeichert und kann später nicht erneut angezeigt werden.</p>
      <button className="primary" onClick={onClose}>Fertig</button>
    </> : <form onSubmit={(event) => void create(event)}>
      <label htmlFor="share-password">Passwort (optional)</label>
      <input id="share-password" type="password" value={password} onChange={(event) => setPassword(event.target.value)} minLength={8} />
      <label htmlFor="share-expiry">Ablauf (optional)</label>
      <input id="share-expiry" type="datetime-local" value={expires} onChange={(event) => setExpires(event.target.value)} />
      <div className="dialog-actions"><button type="button" className="secondary" onClick={onClose}>Abbrechen</button>
        <button className="primary" disabled={busy}>{busy ? "Erstellen …" : "Link erstellen"}</button></div>
    </form>}
  </section></div>;
}

function SharesView({ shares, spaces, token, onChanged, setError }: {
  shares: Share[]; spaces: Space[]; token: string; onChanged: () => void; setError: (message: string) => void;
}) {
  const [acceptingInto, setAcceptingInto] = useState<{ spaceId: string; path: string } | null>(null);
  const [spaceId, setSpaceId] = useState(spaces[0]?.id ?? "");
  const [path, setPath] = useState("Dateiannahme");
  return <><div className="share-tools"><label>Speicherbereich<select value={spaceId} onChange={(event) => setSpaceId(event.target.value)}>
    {spaces.filter((space) => ["owner","manager","editor"].includes(space.role)).map((space) => <option key={space.id} value={space.id}>{space.name}</option>)}</select></label>
    <label>Zielordner<input value={path} onChange={(event) => setPath(event.target.value)} /></label>
    <button className="primary" disabled={!spaceId || !path.trim()} onClick={() => setAcceptingInto({ spaceId, path: path.trim() })}>Dateiannahme-Link</button></div>
  {!shares.length ? <div className="empty"><strong>Noch keine Freigaben</strong><span>Erstelle einen Leselink bei einer Datei oder einen Link zur Dateiannahme.</span></div>
    : <div className="share-list">{shares.map((share) => <article key={share.id} className="share-card">
    <div><strong>{share.target_path}</strong><p>{share.mode === "upload" ? "Dateiannahme" : "Lesen und Download"}
      {share.password_protected ? " · Passwort" : ""}{share.expires_at ? ` · bis ${new Date(share.expires_at).toLocaleString("de-DE")}` : ""}</p></div>
    <span className={share.revoked_at ? "status revoked" : "status"}>{share.revoked_at ? "Widerrufen" : "Aktiv"}</span>
    {!share.revoked_at && <button className="secondary danger-button" onClick={() => void api.revokeShare(token, share.id)
      .then(onChanged).catch((reason) => setError(messageOf(reason)))}>Widerrufen</button>}
  </article>)}</div>}
  {acceptingInto && <PathShareDialog token={token} spaceId={acceptingInto.spaceId} path={acceptingInto.path} mode="upload"
    onClose={() => { setAcceptingInto(null); onChanged(); }} onError={setError} />}</>;
}

function PathShareDialog({ token, spaceId, path, mode, onClose, onError }: {
  token: string; spaceId: string; path: string; mode: "read" | "upload"; onClose: () => void; onError: (message: string) => void;
}) {
  const [createdUrl, setCreatedUrl] = useState("");
  const [busy, setBusy] = useState(false);
  async function create(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); setBusy(true); const data = new FormData(event.currentTarget);
    try {
      const maxMegabytes = Number(data.get("max_megabytes") || 0);
      const result = await api.createShare(token, { space_id: spaceId, target_path: path, mode,
        ...(String(data.get("password") || "") ? { password: String(data.get("password")) } : {}),
        ...(String(data.get("expires") || "") ? { expires_at: new Date(String(data.get("expires"))).toISOString() } : {}),
        ...(mode === "upload" && maxMegabytes > 0 ? { max_upload_size: Math.round(maxMegabytes * 1024 * 1024) } : {}),
      });
      const url = `${location.origin}${location.pathname}${result.url_fragment}`; setCreatedUrl(url);
      await navigator.clipboard?.writeText(url).catch(() => undefined);
    } catch (reason) { onError(messageOf(reason)); } finally { setBusy(false); }
  }
  return <div className="modal-backdrop" role="presentation"><section className="modal" role="dialog" aria-modal="true" aria-labelledby="path-share-title">
    <h2 id="path-share-title">{mode === "upload" ? "Dateiannahme-Link" : "Ordner teilen"}</h2><p className="muted">{path}</p>
    {createdUrl ? <><label>Freigabelink<input value={createdUrl} readOnly onFocus={(event) => event.currentTarget.select()} /></label>
      <p className="muted">Dieser geheime Link wird nur einmal angezeigt.</p><button className="primary" onClick={onClose}>Fertig</button></>
      : <form onSubmit={(event) => void create(event)}><label>Passwort (optional)<input name="password" type="password" minLength={8} /></label>
        <label>Ablauf (optional)<input name="expires" type="datetime-local" /></label>
        {mode === "upload" && <label>Maximale Dateigröße in MB (optional)<input name="max_megabytes" type="number" min="1" /></label>}
        <div className="dialog-actions"><button type="button" className="secondary" onClick={onClose}>Abbrechen</button>
          <button className="primary" disabled={busy}>{busy ? "Erstellen …" : "Link erstellen"}</button></div></form>}
  </section></div>;
}

function SecuritySettings({ session, space, setError, onSpaceChanged }: { session: Session; space?: Space; setError: (message: string) => void; onSpaceChanged: () => void }) {
  const [devices, setDevices] = useState<Device[]>([]);
  const [twoFactor, setTwoFactor] = useState(false);
  const [enrollment, setEnrollment] = useState<{ secret: string; otpauth_uri: string } | null>(null);
  const [codes, setCodes] = useState<string[]>([]);
  async function refresh() {
    try {
      const [deviceResult, status] = await Promise.all([api.devices(session.token), api.twoFactorStatus(session.token)]);
      setDevices(deviceResult.devices); setTwoFactor(status.enabled);
    } catch (reason) { setError(messageOf(reason)); }
  }
  useEffect(() => { void refresh(); }, [session.token]);
  async function begin2fa(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); const data = new FormData(event.currentTarget);
    try { setEnrollment(await api.beginTwoFactor(session.token, String(data.get("password")))); }
    catch (reason) { setError(messageOf(reason)); }
  }
  async function confirm2fa(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); const data = new FormData(event.currentTarget);
    try {
      await api.confirmTwoFactor(session.token, String(data.get("password")), String(data.get("code")));
      setEnrollment(null); setTwoFactor(true);
    } catch (reason) { setError(messageOf(reason)); }
  }
  async function disable2fa(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); const data = new FormData(event.currentTarget);
    try {
      await api.disableTwoFactor(session.token, String(data.get("password")), String(data.get("code")));
      setTwoFactor(false);
    } catch (reason) { setError(messageOf(reason)); }
  }
  async function changePassword(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); const form = event.currentTarget; const data = new FormData(form);
    try {
      await api.changePassword(session.token, String(data.get("current")), String(data.get("next")));
      form.reset();
    } catch (reason) { setError(messageOf(reason)); }
  }
  return <div className="settings-grid">
    {space && ["owner","manager"].includes(space.role) && <section className="settings-card"><h2>Freigaben im Space</h2>
      <p>Editoren dürfen standardmäßig keine Freigabelinks erstellen, auflisten oder widerrufen.</p>
      <label className="checkbox-label"><input type="checkbox" checked={space.editor_sharing}
        onChange={(event) => void api.setSpaceSettings(session.token, space.id, event.target.checked)
          .then(onSpaceChanged).catch((reason) => setError(messageOf(reason)))} /> Editoren dürfen Freigaben verwalten</label></section>}
    {space?.role === "owner" && space.kind === "shared" && <section className="settings-card"><h2>Space löschen</h2>
      <p className="muted">Der Space verschwindet für alle Mitglieder. Aktive Links und Webhooks werden sofort widerrufen.</p>
      <button className="secondary danger-button" onClick={() => {
        const confirmation = window.prompt(`Zum Löschen den Namen „${space.name}“ eingeben`);
        if (confirmation !== null) void api.deleteSpace(session.token, space.id, confirmation)
          .then(onSpaceChanged).catch((reason) => setError(messageOf(reason)));
      }}>Space löschen</button></section>}
    {space && ["owner","manager"].includes(space.role) && <SpaceAccessSettings session={session} space={space} setError={setError} />}
    <section className="settings-card"><h2>Zwei-Faktor-Anmeldung</h2>
      <p className="muted">Status: {twoFactor ? "Aktiv" : "Nicht aktiviert"}. Das Secret liegt mit einem passwortabgeleiteten Schlüssel verschlüsselt vor.</p>
      {!twoFactor && !enrollment && <form onSubmit={(event) => void begin2fa(event)}>
        <label>Passwort bestätigen<input name="password" type="password" required /></label>
        <button className="primary">Einrichtung beginnen</button>
      </form>}
      {enrollment && <form onSubmit={(event) => void confirm2fa(event)}>
        <label>Secret für die Authenticator-App<input value={enrollment.secret} readOnly onFocus={(event) => event.currentTarget.select()} /></label>
        <a href={enrollment.otpauth_uri}>In Authenticator-App öffnen</a>
        <label>Passwort<input name="password" type="password" required /></label>
        <label>Sechsstelliger Code<input name="code" inputMode="numeric" pattern="[0-9]{6}" required /></label>
        <button className="primary">Aktivieren</button>
      </form>}
      {twoFactor && <form onSubmit={(event) => void disable2fa(event)}>
        <label>Passwort<input name="password" type="password" required /></label>
        <label>Aktueller Code<input name="code" inputMode="numeric" pattern="[0-9]{6}" required /></label>
        <button className="secondary danger-button">2FA deaktivieren</button>
      </form>}
    </section>
    <section className="settings-card"><h2>Wiederherstellungscodes</h2>
      <p className="muted">Neue Codes machen alle zuvor erzeugten ungültig. Jeder Code funktioniert genau einmal.</p>
      <button className="secondary" onClick={() => void api.recoveryCodes(session.token).then((result) => setCodes(result.codes)).catch((reason) => setError(messageOf(reason)))}>Neue Codes erzeugen</button>
      {codes.length > 0 && <pre className="recovery-codes">{codes.join("\n")}</pre>}
    </section>
    <section className="settings-card"><h2>Passwort ändern</h2>
      <form onSubmit={(event) => void changePassword(event)}>
        <label>Aktuelles Passwort<input name="current" type="password" required /></label>
        <label>Neues Passwort<input name="next" type="password" minLength={12} required /></label>
        <button className="primary">Passwort ändern</button>
      </form>
      <p className="muted">Andere Sitzungen werden dabei beendet. Dieses Gerät bleibt angemeldet.</p>
    </section>
    <section className="settings-card settings-wide"><h2>Geräte und Sitzungen</h2>
      <div className="device-list">{devices.map((device) => <div key={device.id}>
        <span><strong>{device.name}</strong><small>Zuletzt aktiv: {new Date(device.last_seen_at).toLocaleString("de-DE")} · {device.active_sessions} aktive Sitzung(en)</small></span>
        {device.id === session.device_id ? <span className="status">Dieses Gerät</span> : device.revoked_at ? <span className="status revoked">Widerrufen</span> :
          <button className="secondary danger-button" onClick={() => void api.revokeDevice(session.token, device.id).then(refresh).catch((reason) => setError(messageOf(reason)))}>Widerrufen</button>}
      </div>)}</div>
    </section>
  </div>;
}

function SpaceAccessSettings({ session, space, setError }: { session: Session; space: Space; setError: (message: string) => void }) {
  const [access, setAccess] = useState<SpaceAccess | null>(null);
  async function refresh() {
    try { setAccess(await api.spaceAccess(session.token, space.id)); }
    catch (reason) { setError(messageOf(reason)); }
  }
  useEffect(() => { void refresh(); }, [session.token, space.id]);
  async function addMember(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); const form = event.currentTarget; const data = new FormData(form);
    try { await api.setSpaceMember(session.token, space.id, String(data.get("user")), String(data.get("role"))); form.reset(); await refresh(); }
    catch (reason) { setError(messageOf(reason)); }
  }
  async function addGroup(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); const form = event.currentTarget; const data = new FormData(form);
    try { await api.setSpaceGroup(session.token, space.id, String(data.get("group")), String(data.get("role"))); form.reset(); await refresh(); }
    catch (reason) { setError(messageOf(reason)); }
  }
  if (!access) return <section className="settings-card settings-wide"><h2>Mitglieder</h2><p className="muted">Berechtigungen werden geladen …</p></section>;
  const assignedUsers = new Set(access.members.map((member) => member.user_id));
  const assignedGroups = new Set(access.group_roles.map((group) => group.group_id));
  const users = access.available_users.filter((user) => !assignedUsers.has(user.id));
  const groups = access.available_groups.filter((group) => !assignedGroups.has(group.id));
  const roles = space.role === "owner" ? ["manager","editor","viewer","uploader"] : ["editor","viewer","uploader"];
  return <section className="settings-card settings-wide access-settings"><h2>Mitglieder und Gruppen · {space.name}</h2>
    <div className="access-columns"><div><h3>Direkte Mitglieder</h3><div className="access-list">{access.members.map((member) => <div key={member.user_id}>
      <span><strong>{member.username}</strong><small>{roleLabel(member.role)}</small></span>{member.role !== "owner" && <span className="access-actions">
        <select value={member.role} onChange={(event) => void api.setSpaceMember(session.token, space.id, member.user_id, event.target.value).then(refresh).catch((reason) => setError(messageOf(reason)))}>
          {roles.map((role) => <option key={role} value={role}>{roleLabel(role)}</option>)}</select>
        <button className="text-button danger-button" onClick={() => void api.removeSpaceMember(session.token, space.id, member.user_id).then(refresh).catch((reason) => setError(messageOf(reason)))}>Entfernen</button></span>}
    </div>)}</div>{users.length > 0 && <form className="access-add" onSubmit={(event) => void addMember(event)}>
      <label>Nutzer<select name="user">{users.map((user) => <option key={user.id} value={user.id}>{user.name}</option>)}</select></label>
      <label>Rolle<select name="role">{roles.map((role) => <option key={role} value={role}>{roleLabel(role)}</option>)}</select></label>
      <button className="secondary">Hinzufügen</button></form>}</div>
    <div><h3>Gruppen</h3><div className="access-list">{access.group_roles.map((group) => <div key={group.group_id}>
      <span><strong>{group.name}</strong><small>{roleLabel(group.role)}</small></span><span className="access-actions">
        <select value={group.role} onChange={(event) => void api.setSpaceGroup(session.token, space.id, group.group_id, event.target.value).then(refresh).catch((reason) => setError(messageOf(reason)))}>
          {roles.map((role) => <option key={role} value={role}>{roleLabel(role)}</option>)}</select>
        <button className="text-button danger-button" onClick={() => void api.removeSpaceGroup(session.token, space.id, group.group_id).then(refresh).catch((reason) => setError(messageOf(reason)))}>Entfernen</button></span>
    </div>)}</div>{groups.length > 0 && <form className="access-add" onSubmit={(event) => void addGroup(event)}>
      <label>Gruppe<select name="group">{groups.map((group) => <option key={group.id} value={group.id}>{group.name}</option>)}</select></label>
      <label>Rolle<select name="role">{roles.map((role) => <option key={role} value={role}>{roleLabel(role)}</option>)}</select></label>
      <button className="secondary">Zuweisen</button></form>}</div></div>
  </section>;
}

function AdminView({ session, setError }: { session: Session; setError: (message: string) => void }) {
  const [status, setStatus] = useState<SystemStatus | null>(null);
  const [users, setUsers] = useState<AdminUser[]>([]);
  const [groups, setGroups] = useState<AdminGroup[]>([]);
  const [adminSpaces, setAdminSpaces] = useState<AdminSpace[]>([]);
  const [backup, setBackup] = useState<BackupStatus | null>(null);
  const [backupConfiguration, setBackupConfiguration] = useState<BackupConfiguration | null>(null);
  const [integrity, setIntegrity] = useState("");
  const [backupRunning, setBackupRunning] = useState(false);
  const [mirror, setMirror] = useState<MirrorStatus | null>(null);
  const [mirrorConfiguration, setMirrorConfiguration] = useState<MirrorConfiguration | null>(null);
  const [mirrorRunning, setMirrorRunning] = useState(false);
  const [uploadPolicy, setUploadPolicy] = useState("");
  const [policySaved, setPolicySaved] = useState("");
  async function refresh() {
    try {
      const [system, accounts, groupResult, spaceResult, backupResult, mirrorResult, policy] = await Promise.all([api.systemStatus(session.token), api.adminUsers(session.token), api.adminGroups(session.token), api.adminSpaces(session.token), api.backupStatus(session.token), api.mirrorStatus(session.token), api.uploadPolicy(session.token)]);
      setStatus(system); setUsers(accounts.users); setGroups(groupResult.groups); setAdminSpaces(spaceResult.spaces); setBackup(backupResult.latest); setBackupConfiguration(backupResult.configuration);
      setMirror(mirrorResult.latest); setMirrorConfiguration(mirrorResult.configuration);
      setUploadPolicy(policy.patterns);
    } catch (reason) { setError(messageOf(reason)); }
  }
  useEffect(() => { void refresh(); }, [session.token]);
  async function createUser(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); const form = event.currentTarget; const data = new FormData(form);
    try {
      await api.createUser(session.token, String(data.get("username")), String(data.get("password")), data.get("is_admin") === "on");
      form.reset(); await refresh();
    } catch (reason) { setError(messageOf(reason)); }
  }
  async function setQuota(user: AdminUser) {
    const current = user.quota_bytes ? String(Math.round(user.quota_bytes / 1024 / 1024)) : "";
    const input = window.prompt("Nutzerquote in MiB; leer bedeutet unbegrenzt", current);
    if (input === null) return;
    const parsed = input.trim() ? Number(input) : null;
    if (parsed !== null && (!Number.isFinite(parsed) || parsed < 1)) { setError("Quote muss eine positive Zahl sein."); return; }
    try { await api.setUserQuota(session.token, user.id, parsed === null ? null : Math.round(parsed * 1024 * 1024)); await refresh(); }
    catch (reason) { setError(messageOf(reason)); }
  }
  async function createGroup(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); const form = event.currentTarget; const data = new FormData(form);
    try { await api.createGroup(session.token, String(data.get("name"))); form.reset(); await refresh(); }
    catch (reason) { setError(messageOf(reason)); }
  }
  async function setSpaceQuota(space: AdminSpace) {
    const current = space.quota_bytes ? String(Math.round(space.quota_bytes / 1024 / 1024)) : "";
    const input = window.prompt("Space-Quote in MiB; leer bedeutet unbegrenzt", current);
    if (input === null) return;
    const parsed = input.trim() ? Number(input) : null;
    if (parsed !== null && (!Number.isFinite(parsed) || parsed < 1)) { setError("Quote muss eine positive Zahl sein."); return; }
    try { await api.setSpaceQuota(session.token, space.id, parsed === null ? null : Math.round(parsed * 1024 * 1024)); await refresh(); }
    catch (reason) { setError(messageOf(reason)); }
  }
  async function toggleArchive(space: AdminSpace) {
    const enabled = !space.archive_protected;
    const explanation = enabled
      ? "Archivschutz aktivieren? Normale Clients können dann keine Dateien oder historischen Inhalte löschen."
      : "Archivschutz wirklich aufheben? Danach können normale Lösch- und Aufbewahrungsregeln wieder historische Inhalte entfernen.";
    if (!window.confirm(explanation)) return;
    try { await api.setArchiveProtection(session.token, space.id, enabled); await refresh(); }
    catch (reason) { setError(messageOf(reason)); }
  }
  return <div className="admin-view">
    {status && <section className="metric-grid">
      <article><strong>{status.users}</strong><span>Nutzer</span></article><article><strong>{status.spaces}</strong><span>Spaces</span></article>
      <article><strong>{status.current_files}</strong><span>Aktuelle Dateien</span></article><article><strong>{status.trashed_files}</strong><span>Im Papierkorb</span></article>
      <article><strong>{formatBytes(status.current_bytes)}</strong><span>Aktuelle Inhalte</span></article><article><strong>{formatBytes(status.version_bytes)}</strong><span>Alle Versionen</span></article>
      <article><strong>{formatBytes(status.physical_bytes)}</strong><span>Physischer Blockspeicher</span></article><article><strong>{status.open_uploads}</strong><span>Offene Uploads</span></article>
      <article className={status.expired_uploads ? "metric-warning" : ""}><strong>{status.expired_uploads}</strong><span>Verwaiste Uploads</span></article>
      <article className={status.pending_webhooks ? "metric-warning" : ""}><strong>{status.pending_webhooks}</strong><span>Webhook-Warteschlange</span></article>
    </section>}
    <section className="admin-card"><div><h2>Integrität</h2><p>SQLite-Metadaten werden mit der eingebauten vollständigen Integritätsprüfung geprüft.</p></div>
      <button className="secondary" onClick={() => void api.runIntegrityCheck(session.token).then((result) => setIntegrity(`Bestanden: ${new Date(result.checked_at).toLocaleString("de-DE")}`)).catch((reason) => setError(messageOf(reason)))}>Jetzt prüfen</button>
      <button className="secondary" onClick={() => void downloadDiagnostics(session.token).catch((reason) => setError(messageOf(reason)))}>Diagnose exportieren</button>
      {integrity && <span className="status">{integrity}</span>}</section>
    <section className="admin-card"><div><h2>Letzte Sicherung</h2>{backup ? <p>
      {backup.status === "completed" ? `Erfolgreich · ${backup.entries ?? 0} Einträge · ${formatBytes(backup.bytes ?? 0)}` : backup.status === "running" ? "Läuft gerade" : `Fehlgeschlagen: ${backup.error ?? "Unbekannter Fehler"}`}<br />
      Ziel: {backup.destination}<br />Beginn: {new Date(backup.started_at).toLocaleString("de-DE")}
    </p> : <p>Noch kein interner Backup-Lauf protokolliert. Konfiguriere ein separates Ziel über <code>HEDWIG_BACKUP_DIR</code>.</p>}
      {backupConfiguration && <p>{`Zeitplan: alle ${backupConfiguration.interval_hours} Stunde(n), ${backupConfiguration.keep} Sicherung(en).`}
        {backupConfiguration.same_filesystem && <><br /><strong className="danger-button">Warnung: Daten und Backup-Ziel liegen auf demselben Dateisystem. Ein Laufwerksausfall kann beide Kopien zerstören.</strong></>}</p>}</div>
      <span className="admin-space-actions">{backupConfiguration && <button className="secondary" disabled={backupRunning} onClick={() => { setBackupRunning(true); void api.runBackup(session.token).then(refresh).catch((reason) => setError(messageOf(reason))).finally(() => setBackupRunning(false)); }}>{backupRunning ? "Sicherung läuft …" : "Jetzt sichern"}</button>}
      <span className={backup?.status === "failed" ? "status status-error" : "status"}>{backup?.status === "completed" ? "Geprüft" : backup?.status === "running" ? "Läuft" : backup ? "Fehler" : "Nicht eingerichtet"}</span></span></section>
    <section className="admin-card"><div><h2>Nur-Lese-Spiegel</h2>{mirrorConfiguration ? <p>{mirror ? `${mirror.current_files} aktuelle Dateien · ${formatBytes(mirror.bytes_written)} zusätzlicher Inhalt · Stand ${new Date(mirror.updated_at).toLocaleString("de-DE")}` : "Konfiguriert, aber noch nicht erzeugt."}<br />Ziel: {mirrorConfiguration.destination} · alle {mirrorConfiguration.interval_minutes} Minute(n){mirrorConfiguration.same_filesystem ? " · Warnung: gleiches Dateisystem" : ""}</p> : <p>Optional über <code>HEDWIG_MIRROR_DIR</code> aktivierbar. Der Baum ist nur zum Lesen und kein zweiter Schreibzugang.</p>}</div>{mirrorConfiguration && <button className="secondary" disabled={mirrorRunning} onClick={() => { setMirrorRunning(true); void api.runMirror(session.token).then(setMirror).catch((reason) => setError(messageOf(reason))).finally(() => setMirrorRunning(false)); }}>{mirrorRunning ? "Spiegel wird aktualisiert …" : "Jetzt aktualisieren"}</button>}</section>
    <section className="admin-card"><div><h2>Serverweite Upload-Sperren</h2><p>Ein Muster pro Zeile nach Gitignore-Semantik, zum Beispiel <code>*.exe</code> oder <code>private/**</code>. Diese Regeln gelten zusätzlich für normale Geräte und Gast-Uploads; lokale Ignore-Regeln bleiben davon getrennt.</p>
      <textarea aria-label="Verbotene Upload-Muster" value={uploadPolicy} onChange={(event) => { setUploadPolicy(event.target.value); setPolicySaved(""); }} placeholder={"# Ausführbare Windows-Dateien sperren\n*.exe"} />
      {policySaved && <span className="status" role="status">{policySaved}</span>}</div><button className="secondary" onClick={() => void api.setUploadPolicy(session.token, uploadPolicy).then(() => setPolicySaved("Gespeichert")).catch((reason) => setError(messageOf(reason)))}>Regeln speichern</button></section>
    <section className="admin-card admin-users"><h2>Nutzerkonten</h2><form onSubmit={(event) => void createUser(event)}>
      <label>Benutzername<input name="username" minLength={3} required /></label><label>Startpasswort<input name="password" type="password" minLength={12} required /></label>
      <label className="checkbox-label"><input name="is_admin" type="checkbox" /> Administrator</label><button className="primary">Nutzer anlegen</button></form>
      <div className="admin-user-list">{users.map((user) => <div key={user.id}><span><strong>{user.username}</strong><small>{user.is_admin ? "Administrator" : "Nutzer"} · {user.quota_bytes ? formatBytes(user.quota_bytes) : "Keine Quote"}</small></span>
        <button className="secondary" onClick={() => void setQuota(user)}>Quote ändern</button></div>)}</div></section>
    <section className="admin-card admin-users admin-groups"><h2>Gruppen</h2><form onSubmit={(event) => void createGroup(event)}>
      <label>Gruppenname<input name="name" required maxLength={120} /></label><button className="primary">Gruppe anlegen</button></form>
      <div className="group-list">{groups.map((group) => <article key={group.id}><header><strong>{group.name}</strong><span>{group.member_count} Mitglied(er)</span></header>
        <div className="group-members">{users.map((user) => <label key={user.id} className="checkbox-label"><input type="checkbox" checked={group.member_ids.includes(user.id)}
          onChange={(event) => void api.setGroupMember(session.token, group.id, user.id, event.target.checked).then(refresh).catch((reason) => setError(messageOf(reason)))} /> {user.username}</label>)}</div>
      </article>)}</div></section>
    <section className="admin-card admin-users admin-spaces"><h2>Speicherbereiche</h2><div className="admin-space-list">{adminSpaces.map((space) => <div key={space.id}>
      <span><strong>{space.name}</strong><small>{space.kind === "personal" ? "Persönlich" : "Gemeinsam"} · Owner {space.owner}<br />Aktuell {formatBytes(space.current_bytes)} · Versionen {formatBytes(space.version_bytes)} · {space.quota_bytes ? `Quote ${formatBytes(space.quota_bytes)}` : "Keine Quote"}</small></span>
      <span className="admin-space-actions"><button className="secondary" onClick={() => void setSpaceQuota(space)}>Quote ändern</button>
        <button className={`secondary ${space.archive_protected ? "danger-button" : ""}`} onClick={() => void toggleArchive(space)}>{space.archive_protected ? "Archivschutz aufheben" : "Archiv schützen"}</button></span>
    </div>)}</div></section>
  </div>;
}

function PublicShareScreen({ token }: { token: string }) {
  const [password, setPassword] = useState("");
  const [result, setResult] = useState<{ share: Share; files: CloudFile[] } | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState("");
  const input = useRef<HTMLInputElement>(null);
  async function load(candidate = password) {
    setBusy(true); setError("");
    try { setResult(await api.publicShare(token, candidate)); }
    catch (reason) { setError(candidate ? messageOf(reason) : "Diese Freigabe ist geschützt, abgelaufen oder wurde widerrufen."); }
    finally { setBusy(false); }
  }
  useEffect(() => { void load(""); }, [token]);
  async function upload(file?: File) {
    if (!file) return;
    try { await uploadToShare(token, password, file, (_value, text) => setProgress(text)); setProgress("Datei wurde sicher angenommen."); }
    catch (reason) { setError(messageOf(reason)); setProgress(""); }
    if (input.current) input.current.value = "";
  }
  return <main className="public-share"><section className="public-panel">
    <div className="product-name">Hedwig</div>
    {!result ? <form onSubmit={(event) => { event.preventDefault(); void load(); }}>
      <h1>Geschützte Freigabe</h1><p>Gib das Link-Passwort ein. Abgelaufene oder widerrufene Links bleiben gesperrt.</p>
      {error && <div className="error" role="alert">{error}</div>}
      <label htmlFor="public-password">Passwort</label><input id="public-password" type="password" value={password}
        onChange={(event) => setPassword(event.target.value)} autoFocus />
      <button className="primary" disabled={busy}>{busy ? "Prüfen …" : "Öffnen"}</button>
    </form> : <>
      <h1>{result.share.mode === "upload" ? "Dateien abgeben" : result.share.target_path}</h1>
      {result.share.mode === "upload" ? <div className="drop-zone">
        <p>Vorhandene Dateien sind nicht sichtbar. Gleichnamige Uploads überschreiben nichts.</p>
        <input ref={input} className="visually-hidden" type="file" onChange={(event) => void upload(event.target.files?.[0])} />
        <button className="primary" onClick={() => input.current?.click()}>Datei auswählen</button>
        {progress && <p role="status">{progress}</p>}
      </div> : <div className="public-files">{result.files.map((file) => <article key={file.id}>
        <div><strong>{file.path.split("/").at(-1)}</strong><span>{formatBytes(file.size)}</span></div>
        <button className="secondary" onClick={() => void downloadSharedFile(token, password, file).catch((reason) => setError(messageOf(reason)))}>Herunterladen</button>
      </article>)}</div>}
      {error && <div className="error" role="alert">{error}</div>}
    </>}
  </section></main>;
}

async function refreshContent(session: Session, spaceId: string, includeDeleted: boolean, setFiles: (files: CloudFile[]) => void,
  setFolders: (folders: CloudFolder[]) => void, setError: (error: string) => void) {
  try {
    const [fileResult, folderResult] = await Promise.all([
      api.files(session.token, spaceId, includeDeleted),
      api.folders(session.token, spaceId),
    ]);
    setFiles(fileResult.files); setFolders(folderResult.folders); setError("");
  }
  catch (reason) { setError(messageOf(reason)); }
}
function readSession(): Session | null { try { return JSON.parse(sessionStorage.getItem("cloud-session") || "null"); } catch { return null; } }
function readShareToken(): string | null {
  const match = location.hash.match(/^#\/share\/(cl_[a-f0-9]+)$/);
  return match?.[1] ?? null;
}
function messageOf(reason: unknown): string { return reason instanceof Error ? reason.message : "Unbekannter Fehler"; }
function roleLabel(role: string): string { return ({ owner: "Owner", manager: "Manager", editor: "Bearbeiten", viewer: "Lesen", uploader: "Upload" } as Record<string,string>)[role] || role; }
