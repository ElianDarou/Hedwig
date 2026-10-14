import { blake3 } from "@noble/hashes/blake3.js";
import { bytesToHex } from "@noble/hashes/utils.js";

export type Session = {
  token: string;
  device_id: string;
  username: string;
  personal_space_id: string;
  is_admin: boolean;
};

export type Space = { id: string; name: string; kind: "personal" | "shared"; role: string; editor_sharing: boolean };
export type CloudFile = {
  id: string;
  path: string;
  size: number;
  current_version_id: string | null;
  updated_at: string;
  deleted_at: string | null;
};
export type CloudFolder = { id: string; path: string; created_at: string };
export type FileVersion = {
  id: string;
  size: number;
  created_by: string;
  device_name: string;
  created_at: string;
  pinned_name: string | null;
  current: boolean;
};
export type Share = {
  id: string;
  space_id: string;
  target_path: string;
  file_id: string | null;
  mode: "read" | "upload";
  password_protected: boolean;
  expires_at: string | null;
  revoked_at: string | null;
  max_upload_size: number | null;
  created_at: string;
};
export type Device = {
  id: string; name: string; created_at: string; last_seen_at: string;
  revoked_at: string | null; active_sessions: number;
};
export type Change = {
  sequence: number; event_id: string; space_id: string; object_id: string; kind: string;
  path: string; version_id: string | null; created_at: string;
};
export type AdminUser = { id: string; username: string; is_admin: boolean; created_at: string; disabled: boolean; quota_bytes: number | null };
export type AdminGroup = { id: string; name: string; member_count: number; member_ids: string[]; created_at: string };
export type AdminSpace = { id: string; name: string; kind: "personal" | "shared"; owner: string; quota_bytes: number | null; archive_protected: boolean; current_bytes: number; version_bytes: number };
export type SystemStatus = {
  users: number; spaces: number; current_files: number; trashed_files: number; current_bytes: number;
  version_bytes: number; physical_bytes: number; open_uploads: number; expired_uploads: number;
  pending_webhooks: number; changes: number; schema_version: number;
};
export type BackupStatus = {
  id: string; destination: string; started_at: string; completed_at: string | null;
  status: "running" | "completed" | "failed"; entries: number | null; bytes: number | null; error: string | null;
};
export type BackupConfiguration = { destination: string; interval_hours: number; keep: number; same_filesystem: boolean };
export type MirrorConfiguration = { destination: string; interval_minutes: number; same_filesystem: boolean };
export type MirrorStatus = { destination: string; updated_at: string; current_files: number; bytes_written: number };
export type FolderRestorePreview = {
  space_id: string; folder_path: string; target_at: string;
  actions: { file_id: string; path: string; action: "delete" | "restore" | "move" | "restore_version" }[];
  unavailable: string[];
};
export type SpaceAccess = {
  members: { user_id: string; username: string; role: string }[];
  group_roles: { group_id: string; name: string; role: string }[];
  available_users: { id: string; name: string }[];
  available_groups: { id: string; name: string }[];
};

type ApiErrorBody = { error?: { message?: string } };

export class ApiError extends Error {
  constructor(public readonly status: number, message: string) {
    super(message);
    this.name = "ApiError";
  }
}

async function request<T>(path: string, init: RequestInit = {}, token?: string): Promise<T> {
  const headers = new Headers(init.headers);
  if (token) headers.set("Authorization", `Bearer ${token}`);
  if (init.body && !(init.body instanceof Blob) && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }
  const response = await fetch(path, { ...init, headers });
  if (!response.ok) {
    const body = (await response.json().catch(() => ({}))) as ApiErrorBody;
    throw new ApiError(response.status, body.error?.message ?? `Anfrage fehlgeschlagen (${response.status})`);
  }
  if (response.status === 204) return undefined as T;
  return response.json() as Promise<T>;
}

async function shareRequest<T>(path: string, shareToken: string, password?: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  headers.set("Authorization", `Share ${shareToken}`);
  if (password) headers.set("X-Share-Password", password);
  if (init.body && !(init.body instanceof Blob) && !headers.has("Content-Type")) headers.set("Content-Type", "application/json");
  const response = await fetch(path, { ...init, headers });
  if (!response.ok) {
    const body = (await response.json().catch(() => ({}))) as ApiErrorBody;
    throw new Error(body.error?.message ?? `Anfrage fehlgeschlagen (${response.status})`);
  }
  if (response.status === 204) return undefined as T;
  return response.json() as Promise<T>;
}

export const api = {
  setupStatus: () => request<{ setup_required: boolean }>("/v1/setup/status"),
  setup: (username: string, password: string) => request<Session>("/v1/setup", {
    method: "POST",
    body: JSON.stringify({ username, password, device_name: browserName() }),
  }),
  login: (username: string, password: string, otp?: string) => request<Session>("/v1/auth/login", {
    method: "POST",
    body: JSON.stringify({ username, password, device_name: browserName(), ...(otp ? { otp } : {}) }),
  }),
  recover: (username: string, recoveryCode: string) => request<Session>("/v1/auth/recover", {
    method: "POST", body: JSON.stringify({ username, recovery_code: recoveryCode, device_name: browserName() }),
  }),
  logout: (token: string) => request<void>("/v1/auth/logout", { method: "POST" }, token),
  spaces: (token: string) => request<{ spaces: Space[] }>("/v1/spaces", {}, token),
  createSpace: (token: string, name: string) => request<Space>("/v1/spaces", {
    method: "POST", body: JSON.stringify({ name }),
  }, token),
  deleteSpace: (token: string, spaceId: string, name: string) => request<void>(`/v1/spaces/${spaceId}/delete`, {
    method: "POST", body: JSON.stringify({ name }),
  }, token),
  setSpaceSettings: (token: string, spaceId: string, editorSharing: boolean) => request<void>(`/v1/spaces/${spaceId}/settings`, {
    method: "PUT", body: JSON.stringify({ editor_sharing: editorSharing }),
  }, token),
  spaceAccess: (token: string, spaceId: string) => request<SpaceAccess>(`/v1/spaces/${spaceId}/access`, {}, token),
  setSpaceMember: (token: string, spaceId: string, userId: string, role: string) => request<void>(`/v1/spaces/${spaceId}/members/${userId}`, {
    method: "PUT", body: JSON.stringify({ role }),
  }, token),
  removeSpaceMember: (token: string, spaceId: string, userId: string) => request<void>(`/v1/spaces/${spaceId}/members/${userId}`, { method: "DELETE" }, token),
  setSpaceGroup: (token: string, spaceId: string, groupId: string, role: string) => request<void>(`/v1/spaces/${spaceId}/groups/${groupId}`, {
    method: "PUT", body: JSON.stringify({ role }),
  }, token),
  removeSpaceGroup: (token: string, spaceId: string, groupId: string) => request<void>(`/v1/spaces/${spaceId}/groups/${groupId}`, { method: "DELETE" }, token),
  files: (token: string, spaceId: string, includeDeleted = false) =>
    request<{ files: CloudFile[] }>(`/v1/spaces/${spaceId}/files${includeDeleted ? "?include_deleted=true" : ""}`, {}, token),
  folders: (token: string, spaceId: string) =>
    request<{ folders: CloudFolder[] }>(`/v1/spaces/${spaceId}/folders`, {}, token),
  createFolder: (token: string, spaceId: string, path: string) =>
    request<CloudFolder>(`/v1/spaces/${spaceId}/folders`, {
      method: "POST", body: JSON.stringify({ path }),
    }, token),
  deleteFolder: (token: string, folderId: string) =>
    request<{ deleted_files: number }>(`/v1/folders/${folderId}`, { method: "DELETE" }, token),
  moveFolder: (token: string, folderId: string, path: string) =>
    request<CloudFolder>(`/v1/folders/${folderId}/move`, {
      method: "POST", body: JSON.stringify({ path }),
    }, token),
  folderRestorePreview: (token: string, spaceId: string, path: string, at: string) =>
    request<FolderRestorePreview>(`/v1/spaces/${spaceId}/folder-restore?path=${encodeURIComponent(path)}&at=${encodeURIComponent(at)}`, {}, token),
  restoreFolder: (token: string, spaceId: string, path: string, at: string) =>
    request<{ restored_files: number; deleted_files: number }>(`/v1/spaces/${spaceId}/folder-restore`, {
      method: "POST", body: JSON.stringify({ path, at }),
    }, token),
  changes: (token: string, after = 0, limit = 250) =>
    request<{ changes: Change[]; next_cursor: number }>(`/v1/changes?after=${after}&limit=${limit}`, {}, token),
  adminUsers: (token: string) => request<{ users: AdminUser[] }>("/v1/admin/users", {}, token),
  adminGroups: (token: string) => request<{ groups: AdminGroup[] }>("/v1/admin/groups", {}, token),
  adminSpaces: (token: string) => request<{ spaces: AdminSpace[] }>("/v1/admin/spaces", {}, token),
  createGroup: (token: string, name: string) => request<AdminGroup>("/v1/admin/groups", {
    method: "POST", body: JSON.stringify({ name }),
  }, token),
  setGroupMember: (token: string, groupId: string, userId: string, enabled: boolean) => request<void>(`/v1/admin/groups/${groupId}/members/${userId}`, {
    method: enabled ? "PUT" : "DELETE",
  }, token),
  createUser: (token: string, username: string, password: string, isAdmin: boolean) => request<AdminUser>("/v1/admin/users", {
    method: "POST", body: JSON.stringify({ username, password, is_admin: isAdmin }),
  }, token),
  setUserQuota: (token: string, userId: string, quotaBytes: number | null) => request<void>(`/v1/admin/users/${userId}/quota`, {
    method: "PUT", body: JSON.stringify({ quota_bytes: quotaBytes }),
  }, token),
  setSpaceQuota: (token: string, spaceId: string, quotaBytes: number | null) => request<void>(`/v1/admin/spaces/${spaceId}/quota`, {
    method: "PUT", body: JSON.stringify({ quota_bytes: quotaBytes }),
  }, token),
  setArchiveProtection: (token: string, spaceId: string, enabled: boolean) => request<void>(`/v1/admin/spaces/${spaceId}/archive-protection`, {
    method: "PUT", body: JSON.stringify({ enabled }),
  }, token),
  systemStatus: (token: string) => request<SystemStatus>("/v1/admin/system", {}, token),
  backupStatus: (token: string) => request<{ latest: BackupStatus | null; configuration: BackupConfiguration | null }>("/v1/admin/backup-status", {}, token),
  runBackup: (token: string) => request<{ status: string; entries: number; bytes: number; created_at: string }>("/v1/admin/backup-run", { method: "POST" }, token),
  mirrorStatus: (token: string) => request<{ latest: MirrorStatus | null; configuration: MirrorConfiguration | null }>("/v1/admin/mirror-status", {}, token),
  runMirror: (token: string) => request<MirrorStatus>("/v1/admin/mirror-run", { method: "POST" }, token),
  uploadPolicy: (token: string) => request<{ patterns: string }>("/v1/admin/upload-policy", {}, token),
  setUploadPolicy: (token: string, patterns: string) => request<{ patterns: string }>("/v1/admin/upload-policy", {
    method: "PUT", body: JSON.stringify({ patterns }),
  }, token),
  runIntegrityCheck: (token: string) => request<{ status: string; checked_at: string }>("/v1/admin/integrity", { method: "POST" }, token),
  versions: (token: string, fileId: string) =>
    request<{ versions: FileVersion[] }>(`/v1/files/${fileId}/versions`, {}, token),
  move: (token: string, fileId: string, path: string) => request(`/v1/files/${fileId}/move`, {
    method: "POST", body: JSON.stringify({ path }),
  }, token),
  copy: (token: string, fileId: string, path: string, spaceId?: string) => request(`/v1/files/${fileId}/copy`, {
    method: "POST", body: JSON.stringify({ path, ...(spaceId ? { space_id: spaceId } : {}) }),
  }, token),
  remove: (token: string, fileId: string) => request(`/v1/files/${fileId}`, { method: "DELETE" }, token),
  restore: (token: string, fileId: string) => request(`/v1/files/${fileId}/restore`, { method: "POST" }, token),
  restoreVersion: (token: string, fileId: string, versionId: string) =>
    request(`/v1/files/${fileId}/versions/${versionId}/restore`, { method: "POST" }, token),
  pinVersion: (token: string, fileId: string, versionId: string, name: string | null) =>
    request<void>(`/v1/files/${fileId}/versions/${versionId}/pin`, {
      method: "PUT", body: JSON.stringify({ name }),
    }, token),
  shares: (token: string) => request<{ shares: Share[] }>("/v1/shares", {}, token),
  createShare: (token: string, input: {
    space_id: string; target_path: string; mode: "read" | "upload"; password?: string;
    expires_at?: string; max_upload_size?: number;
  }) => request<{ share: Share; token: string; url_fragment: string }>("/v1/shares", {
    method: "POST", body: JSON.stringify(input),
  }, token),
  revokeShare: (token: string, shareId: string) => request<void>(`/v1/shares/${shareId}`, { method: "DELETE" }, token),
  publicShare: (shareToken: string, password?: string) =>
    shareRequest<{ share: Share; files: CloudFile[] }>("/v1/public/share", shareToken, password),
  devices: (token: string) => request<{ devices: Device[] }>("/v1/devices", {}, token),
  revokeDevice: (token: string, deviceId: string) => request<void>(`/v1/devices/${deviceId}`, { method: "DELETE" }, token),
  changePassword: (token: string, currentPassword: string, newPassword: string) => request<void>("/v1/auth/password", {
    method: "PUT", body: JSON.stringify({ current_password: currentPassword, new_password: newPassword }),
  }, token),
  recoveryCodes: (token: string) => request<{ codes: string[] }>("/v1/auth/recovery-codes", { method: "POST" }, token),
  twoFactorStatus: (token: string) => request<{ enabled: boolean }>("/v1/auth/2fa", {}, token),
  beginTwoFactor: (token: string, password: string) => request<{ secret: string; otpauth_uri: string }>("/v1/auth/2fa/begin", {
    method: "POST", body: JSON.stringify({ password }),
  }, token),
  confirmTwoFactor: (token: string, password: string, code: string) => request<void>("/v1/auth/2fa/confirm", {
    method: "POST", body: JSON.stringify({ password, code }),
  }, token),
  disableTwoFactor: (token: string, password: string, code: string) => request<void>("/v1/auth/2fa/disable", {
    method: "POST", body: JSON.stringify({ password, code }),
  }, token),
};

export async function downloadDiagnostics(token: string): Promise<void> {
  const response = await fetch("/v1/admin/diagnostics", { headers: { Authorization: `Bearer ${token}` } });
  if (!response.ok) throw new Error("Diagnoseexport fehlgeschlagen");
  saveBlob(await response.blob(), "hedwig-diagnostics.json");
}

const BLOCK_SIZE = 1024 * 1024;

export async function uploadFile(
  token: string,
  spaceId: string,
  file: File,
  onProgress: (value: number, text: string) => void,
  targetPath?: string,
): Promise<void> {
  const blocks: { hash: string; size: number; blob: Blob }[] = [];
  for (let offset = 0; offset < file.size; offset += BLOCK_SIZE) {
    const blob = file.slice(offset, Math.min(offset + BLOCK_SIZE, file.size));
    const bytes = new Uint8Array(await blob.arrayBuffer());
    blocks.push({ hash: bytesToHex(blake3(bytes)), size: bytes.length, blob });
    onProgress(Math.min(20, (offset / Math.max(file.size, 1)) * 20), "Datei wird geprüft …");
  }

  const started = await request<{ upload_id: string; missing_blocks: string[] }>(
    `/v1/spaces/${spaceId}/uploads`,
    {
      method: "POST",
      body: JSON.stringify({
        path: targetPath || file.webkitRelativePath || file.name,
        size: file.size,
        blocks: blocks.map(({ hash, size }) => ({ hash, size })),
        idempotency_key: crypto.randomUUID(),
      }),
    },
    token,
  );

  const missing = new Set(started.missing_blocks);
  let sent = 0;
  const bytesToSend = blocks.filter((block) => missing.has(block.hash)).reduce((sum, block) => sum + block.size, 0);
  for (const block of blocks) {
    if (!missing.has(block.hash)) continue;
    await request<void>(`/v1/uploads/${started.upload_id}/blocks/${block.hash}`, {
      method: "PUT",
      body: block.blob,
      headers: { "Content-Type": "application/octet-stream" },
    }, token);
    sent += block.size;
    onProgress(20 + (sent / Math.max(bytesToSend, 1)) * 75, `${formatBytes(sent)} übertragen`);
  }
  onProgress(96, "Version wird bestätigt …");
  await request(`/v1/uploads/${started.upload_id}/commit`, { method: "POST" }, token);
  onProgress(100, "Upload abgeschlossen");
}

export async function downloadFile(token: string, file: CloudFile): Promise<void> {
  return downloadFrom(token, `/v1/files/${file.id}/content`, file.path);
}

export async function fetchFileBlob(token: string, file: CloudFile): Promise<Blob> {
  const response = await fetch(`/v1/files/${file.id}/content`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  if (!response.ok) throw new Error("Vorschau konnte nicht geladen werden");
  return response.blob();
}

export async function downloadVersion(token: string, file: CloudFile, versionId: string): Promise<void> {
  return downloadFrom(token, `/v1/files/${file.id}/versions/${versionId}/content`, file.path);
}

export async function fetchVersionText(token: string, fileId: string, versionId: string): Promise<string> {
  const response = await fetch(`/v1/files/${fileId}/versions/${versionId}/content`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  if (!response.ok) throw new Error("Version konnte nicht geladen werden");
  return response.text();
}

export async function downloadFolderArchive(token: string, spaceId: string, path: string): Promise<void> {
  const response = await fetch(`/v1/spaces/${spaceId}/archive?path=${encodeURIComponent(path)}`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  if (!response.ok) throw new Error("Archiv-Download fehlgeschlagen");
  saveBlob(await response.blob(), `${path.split("/").at(-1) || "Cloud-Dateien"}.zip`);
}

export async function downloadSharedFile(shareToken: string, password: string, file: CloudFile): Promise<void> {
  const response = await fetch(`/v1/public/share/files/${file.id}/content`, {
    headers: { Authorization: `Share ${shareToken}`, ...(password ? { "X-Share-Password": password } : {}) },
  });
  if (!response.ok) throw new Error("Download fehlgeschlagen");
  saveBlob(await response.blob(), file.path);
}

async function downloadFrom(token: string, path: string, filePath: string): Promise<void> {
  const response = await fetch(path, { headers: { Authorization: `Bearer ${token}` } });
  if (!response.ok) throw new Error("Download fehlgeschlagen");
  saveBlob(await response.blob(), filePath);
}

function saveBlob(blob: Blob, filePath: string): void {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filePath.split("/").at(-1) || "Download";
  anchor.click();
  URL.revokeObjectURL(url);
}

export async function uploadToShare(
  shareToken: string,
  password: string,
  file: File,
  onProgress: (value: number, text: string) => void,
): Promise<void> {
  const blocks: { hash: string; size: number; blob: Blob }[] = [];
  for (let offset = 0; offset < file.size; offset += BLOCK_SIZE) {
    const blob = file.slice(offset, Math.min(offset + BLOCK_SIZE, file.size));
    const bytes = new Uint8Array(await blob.arrayBuffer());
    blocks.push({ hash: bytesToHex(blake3(bytes)), size: bytes.length, blob });
    onProgress(Math.min(20, (offset / Math.max(file.size, 1)) * 20), "Datei wird geprüft …");
  }
  const started = await shareRequest<{ upload_id: string; missing_blocks: string[] }>(
    "/v1/public/share/uploads", shareToken, password,
    { method: "POST", body: JSON.stringify({ path: file.name, size: file.size,
      blocks: blocks.map(({ hash, size }) => ({ hash, size })), idempotency_key: crypto.randomUUID() }) },
  );
  const missing = new Set(started.missing_blocks);
  let sent = 0;
  const total = blocks.filter((block) => missing.has(block.hash)).reduce((sum, block) => sum + block.size, 0);
  for (const block of blocks) {
    if (!missing.has(block.hash)) continue;
    await shareRequest<void>(`/v1/public/share/uploads/${started.upload_id}/blocks/${block.hash}`, shareToken, password,
      { method: "PUT", body: block.blob, headers: { "Content-Type": "application/octet-stream" } });
    sent += block.size;
    onProgress(20 + (sent / Math.max(total, 1)) * 75, `${formatBytes(sent)} übertragen`);
  }
  await shareRequest(`/v1/public/share/uploads/${started.upload_id}/commit`, shareToken, password, { method: "POST" });
  onProgress(100, "Upload abgeschlossen");
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
}

function browserName(): string {
  const platform = (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData?.platform
    || navigator.platform || "Unbekannt";
  return `Web · ${platform}`;
}
