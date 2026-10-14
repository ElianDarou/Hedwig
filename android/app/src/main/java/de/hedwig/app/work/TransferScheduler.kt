package de.hedwig.app.work

import android.content.Context
import android.net.Uri
import androidx.work.Constraints
import androidx.work.Data
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import de.hedwig.app.data.AppSettings
import java.util.UUID
import java.util.concurrent.TimeUnit

object TransferScheduler {
    fun upload(context: Context, uri: Uri, spaceId: String, path: String): UUID {
        runCatching {
            context.contentResolver.takePersistableUriPermission(uri, android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        val key = "android-${UUID.randomUUID()}"
        val request = OneTimeWorkRequestBuilder<UploadWorker>()
            .setConstraints(networkConstraints(context))
            .setInputData(
                Data.Builder()
                    .putString("uri", uri.toString())
                    .putString("space_id", spaceId)
                    .putString("path", path)
                    .putString("idempotency_key", key)
                    .build(),
            )
            .build()
        WorkManager.getInstance(context).enqueue(request)
        return request.id
    }

    fun keepOffline(context: Context, spaceId: String, fileId: String, name: String): UUID {
        val request = OneTimeWorkRequestBuilder<OfflineDownloadWorker>()
            .setConstraints(networkConstraints(context))
            .setInputData(
                Data.Builder().putString("space_id", spaceId).putString("file_id", fileId)
                    .putString("name", name).build(),
            ).build()
        WorkManager.getInstance(context).enqueue(request)
        return request.id
    }

    fun configureCameraBackup(context: Context) {
        val settings = AppSettings(context)
        val manager = WorkManager.getInstance(context)
        if (!settings.cameraBackupEnabled || settings.cameraSpaceId == null) {
            manager.cancelUniqueWork(CAMERA_WORK)
            return
        }
        val request = PeriodicWorkRequestBuilder<CameraBackupWorker>(15, TimeUnit.MINUTES)
            .setConstraints(networkConstraints(context, includeCharging = true))
            .build()
        manager.enqueueUniquePeriodicWork(CAMERA_WORK, ExistingPeriodicWorkPolicy.UPDATE, request)
    }

    fun syncNow(context: Context) = WorkManager.getInstance(context).enqueue(
        OneTimeWorkRequestBuilder<CameraBackupWorker>()
            .setConstraints(networkConstraints(context, includeCharging = false)).build(),
    )

    fun configureTreeSync(context: Context) {
        val settings = AppSettings(context)
        val manager = WorkManager.getInstance(context)
        if (settings.treeSyncUri == null || settings.treeSyncSpaceId == null) {
            manager.cancelUniqueWork(TREE_WORK)
            return
        }
        val request = PeriodicWorkRequestBuilder<TreeSyncWorker>(15, TimeUnit.MINUTES)
            .setConstraints(networkConstraints(context))
            .build()
        manager.enqueueUniquePeriodicWork(TREE_WORK, ExistingPeriodicWorkPolicy.UPDATE, request)
    }

    fun syncTreeNow(context: Context) = WorkManager.getInstance(context).enqueue(
        OneTimeWorkRequestBuilder<TreeSyncWorker>().setConstraints(networkConstraints(context)).build(),
    )

    private fun networkConstraints(context: Context, includeCharging: Boolean = true): Constraints {
        val settings = AppSettings(context)
        return Constraints.Builder()
            .setRequiredNetworkType(if (settings.wifiOnly) NetworkType.UNMETERED else NetworkType.CONNECTED)
            .setRequiresCharging(includeCharging && settings.chargingOnly)
            .build()
    }

    private const val CAMERA_WORK = "cloud-camera-backup"
    private const val TREE_WORK = "cloud-saf-tree-sync"
}
