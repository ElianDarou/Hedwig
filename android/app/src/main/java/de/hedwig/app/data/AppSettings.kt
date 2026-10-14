package de.hedwig.app.data

import android.content.Context
import androidx.core.content.edit
import org.json.JSONObject

data class TreeSyncRecord(val fileId: String, val versionId: String, val signature: String)

class AppSettings(context: Context) {
    private val preferences = context.getSharedPreferences("cloud-settings", Context.MODE_PRIVATE)

    var wifiOnly: Boolean
        get() = preferences.getBoolean("wifi_only", true)
        set(value) = preferences.edit { putBoolean("wifi_only", value) }
    var chargingOnly: Boolean
        get() = preferences.getBoolean("charging_only", false)
        set(value) = preferences.edit { putBoolean("charging_only", value) }
    var maximumUploadBytes: Long
        get() = preferences.getLong("maximum_upload_bytes", 2L * 1024 * 1024 * 1024)
        set(value) = preferences.edit { putLong("maximum_upload_bytes", value) }
    var cameraBackupEnabled: Boolean
        get() = preferences.getBoolean("camera_backup", false)
        set(value) = preferences.edit { putBoolean("camera_backup", value) }
    var cameraSpaceId: String?
        get() = preferences.getString("camera_space", null)
        set(value) = preferences.edit { putString("camera_space", value) }
    var cameraMediaFolders: Set<String>
        get() = preferences.getStringSet("camera_folders", setOf("DCIM/Camera/")) ?: setOf("DCIM/Camera/")
        set(value) = preferences.edit { putStringSet("camera_folders", value) }
    var treeSyncUri: String?
        get() = preferences.getString("tree_sync_uri", null)
        set(value) = preferences.edit { putString("tree_sync_uri", value) }
    var treeSyncSpaceId: String?
        get() = preferences.getString("tree_sync_space", null)
        set(value) = preferences.edit { putString("tree_sync_space", value) }
    var treeSyncPrefix: String
        get() = preferences.getString("tree_sync_prefix", "Android-Ordner") ?: "Android-Ordner"
        set(value) = preferences.edit { putString("tree_sync_prefix", value) }

    fun treeRecord(uri: String): TreeSyncRecord? = runCatching {
        val value = JSONObject(preferences.getString("tree_sync_records", "{}") ?: "{}").getJSONObject(uri)
        TreeSyncRecord(value.getString("file_id"), value.getString("version_id"), value.getString("signature"))
    }.getOrNull()

    fun setTreeRecord(uri: String, record: TreeSyncRecord) {
        val root = runCatching { JSONObject(preferences.getString("tree_sync_records", "{}") ?: "{}") }.getOrElse { JSONObject() }
        root.put(uri, JSONObject().put("file_id", record.fileId).put("version_id", record.versionId).put("signature", record.signature))
        preferences.edit { putString("tree_sync_records", root.toString()) }
    }

    fun markCameraItem(identity: String) {
        val current = preferences.getStringSet("camera_done", emptySet()).orEmpty().toMutableSet()
        current += identity
        preferences.edit { putStringSet("camera_done", current.toList().takeLast(10_000).toSet()) }
    }

    fun cameraItemDone(identity: String) =
        preferences.getStringSet("camera_done", emptySet()).orEmpty().contains(identity)
}
