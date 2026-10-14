package de.hedwig.app.work

import android.content.Context
import android.os.Build
import android.provider.MediaStore
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import de.hedwig.app.data.AppSettings
import de.hedwig.app.data.CloudApi
import de.hedwig.app.data.SecretStore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.util.UUID

class CameraBackupWorker(context: Context, parameters: WorkerParameters) :
    CoroutineWorker(context, parameters) {
    override suspend fun doWork(): Result = withContext(Dispatchers.IO) {
        val settings = AppSettings(applicationContext)
        if (!settings.cameraBackupEnabled) return@withContext Result.success()
        val space = settings.cameraSpaceId ?: return@withContext Result.failure(
            workDataOf("error" to "Kein Ziel-Space für Kamera-Backup gewählt"),
        )
        val account = SecretStore(applicationContext).load() ?: return@withContext Result.failure()
        val api = CloudApi(account)
        var uploaded = 0
        try {
            for (media in mediaItems(settings.cameraMediaFolders)) {
                if (settings.cameraItemDone(media.identity)) continue
                if (media.size > settings.maximumUploadBytes) continue
                val temporary = File(applicationContext.cacheDir, "camera/${media.identity.hashCode()}")
                temporary.parentFile?.mkdirs()
                val input = applicationContext.contentResolver.openInputStream(media.uri) ?: continue
                input.use { source -> temporary.outputStream().use { source.copyTo(it) } }
                api.upload(
                    space,
                    "Kamera/${media.displayName}",
                    temporary,
                    "android-camera-${UUID.nameUUIDFromBytes(media.identity.encodeToByteArray())}",
                )
                temporary.delete()
                settings.markCameraItem(media.identity)
                uploaded += 1
                setProgress(workDataOf("uploaded" to uploaded, "current" to media.displayName))
            }
            Result.success(workDataOf("uploaded" to uploaded))
        } catch (error: SecurityException) {
            Result.failure(workDataOf("error" to "Android hat den Zugriff auf die Medienordner nicht erlaubt"))
        } catch (error: Exception) {
            if (runAttemptCount < 8) Result.retry()
            else Result.failure(workDataOf("error" to (error.message ?: "Kamera-Backup fehlgeschlagen")))
        }
    }

    private fun mediaItems(folders: Set<String>): List<MediaItem> {
        val projection = arrayOf(
            MediaStore.Files.FileColumns._ID,
            MediaStore.Files.FileColumns.DISPLAY_NAME,
            MediaStore.Files.FileColumns.SIZE,
            MediaStore.Files.FileColumns.DATE_MODIFIED,
            MediaStore.Files.FileColumns.RELATIVE_PATH,
            MediaStore.Files.FileColumns.MEDIA_TYPE,
        )
        val selection = "${MediaStore.Files.FileColumns.MEDIA_TYPE} IN (?, ?)"
        val arguments = arrayOf(
            MediaStore.Files.FileColumns.MEDIA_TYPE_IMAGE.toString(),
            MediaStore.Files.FileColumns.MEDIA_TYPE_VIDEO.toString(),
        )
        val collection = MediaStore.Files.getContentUri(
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                MediaStore.VOLUME_EXTERNAL
            } else {
                "external"
            },
        )
        val output = mutableListOf<MediaItem>()
        applicationContext.contentResolver.query(
            collection,
            projection,
            selection,
            arguments,
            "${MediaStore.Files.FileColumns.DATE_MODIFIED} ASC",
        )?.use { cursor ->
            val idColumn = cursor.getColumnIndexOrThrow(MediaStore.Files.FileColumns._ID)
            val nameColumn = cursor.getColumnIndexOrThrow(MediaStore.Files.FileColumns.DISPLAY_NAME)
            val sizeColumn = cursor.getColumnIndexOrThrow(MediaStore.Files.FileColumns.SIZE)
            val modifiedColumn = cursor.getColumnIndexOrThrow(MediaStore.Files.FileColumns.DATE_MODIFIED)
            val folderColumn = cursor.getColumnIndexOrThrow(MediaStore.Files.FileColumns.RELATIVE_PATH)
            while (cursor.moveToNext()) {
                val folder = cursor.getString(folderColumn) ?: ""
                if (folder !in folders) continue
                val id = cursor.getLong(idColumn)
                val name = cursor.getString(nameColumn) ?: "Medium-$id"
                val size = cursor.getLong(sizeColumn)
                val modified = cursor.getLong(modifiedColumn)
                output += MediaItem(
                    android.content.ContentUris.withAppendedId(collection, id),
                    name,
                    size,
                    "$id:$size:$modified",
                )
            }
        }
        return output
    }

    private data class MediaItem(
        val uri: android.net.Uri,
        val displayName: String,
        val size: Long,
        val identity: String,
    )
}
