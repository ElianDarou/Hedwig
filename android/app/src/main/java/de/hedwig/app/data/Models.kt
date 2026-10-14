package de.hedwig.app.data

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
data class Session(
    val token: String,
    @SerialName("user_id") val userId: String,
    val username: String,
    @SerialName("device_id") val deviceId: String,
    @SerialName("personal_space_id") val personalSpaceId: String,
    @SerialName("is_admin") val isAdmin: Boolean,
)

@Serializable
data class Space(val id: String, val name: String, val kind: String, val role: String)

@Serializable
data class SpaceList(val spaces: List<Space>)

@Serializable
data class CloudFile(
    val id: String,
    val path: String,
    val size: Long,
    @SerialName("current_version_id") val currentVersionId: String? = null,
    @SerialName("updated_at") val updatedAt: String,
    @SerialName("deleted_at") val deletedAt: String? = null,
)

@Serializable
data class FileList(val files: List<CloudFile>)

@Serializable
data class BlockDescriptor(val hash: String, val size: Int)

@Serializable
data class UploadStart(
    @SerialName("upload_id") val uploadId: String,
    @SerialName("missing_blocks") val missingBlocks: List<String>,
    @SerialName("expires_at") val expiresAt: String,
    val status: String,
)

@Serializable
data class UploadCommit(
    @SerialName("file_id") val fileId: String,
    @SerialName("version_id") val versionId: String,
    val path: String,
    val conflict: Boolean,
    @SerialName("change_sequence") val changeSequence: Long,
)

@Serializable
data class FileManifest(
    @SerialName("file_id") val fileId: String,
    @SerialName("version_id") val versionId: String,
    val path: String,
    val size: Long,
    val blocks: List<BlockDescriptor>,
)

@Serializable
data class FileVersion(
    val id: String,
    val size: Long,
    @SerialName("created_by") val createdBy: String,
    @SerialName("device_name") val deviceName: String,
    @SerialName("created_at") val createdAt: String,
    @SerialName("pinned_name") val pinnedName: String? = null,
    val current: Boolean,
)

@Serializable
data class VersionList(val versions: List<FileVersion>)

@Serializable
data class ShareInfo(val id: String, @SerialName("target_path") val targetPath: String, val mode: String)

@Serializable
data class CreatedShare(val share: ShareInfo, val token: String, @SerialName("url_fragment") val urlFragment: String)

@Serializable
data class Share(
    val id: String,
    @SerialName("space_id") val spaceId: String,
    @SerialName("target_path") val targetPath: String,
    val mode: String,
    @SerialName("password_protected") val passwordProtected: Boolean,
    @SerialName("expires_at") val expiresAt: String? = null,
    @SerialName("revoked_at") val revokedAt: String? = null,
    @SerialName("max_upload_size") val maxUploadSize: Long? = null,
    @SerialName("created_at") val createdAt: String,
)

@Serializable
data class ShareList(val shares: List<Share>)

data class CloudAccount(val server: String, val session: Session)
