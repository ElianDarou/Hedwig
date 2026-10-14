package de.hedwig.app.work

import android.content.Context
import androidx.core.content.edit
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import de.hedwig.app.data.CloudApi
import de.hedwig.app.data.CloudApiException
import de.hedwig.app.data.SecretStore
import java.io.File

class OfflineDownloadWorker(context: Context, parameters: WorkerParameters) :
    CoroutineWorker(context, parameters) {
    override suspend fun doWork(): Result {
        val account = SecretStore(applicationContext).load() ?: return Result.failure()
        val space = inputData.getString("space_id") ?: return Result.failure()
        val fileId = inputData.getString("file_id") ?: return Result.failure()
        val name = inputData.getString("name") ?: return Result.failure()
        val safeName = name.substringAfterLast('/').replace(Regex("[^A-Za-z0-9._ -]"), "_")
        val destination = File(applicationContext.filesDir, "offline/$space/$fileId/$safeName")
        return try {
            CloudApi(account).download(fileId, destination)
            applicationContext.getSharedPreferences("offline-files", Context.MODE_PRIVATE).edit {
                putString(fileId, destination.absolutePath)
            }
            Result.success(workDataOf("path" to destination.absolutePath))
        } catch (error: CloudApiException) {
            if (error.status >= 500 || error.status == 429) Result.retry()
            else Result.failure(workDataOf("error" to error.message))
        } catch (error: Exception) {
            if (runAttemptCount < 8) Result.retry() else Result.failure(
                workDataOf("error" to (error.message ?: "Download fehlgeschlagen")),
            )
        }
    }
}
