package de.hedwig.app.work

import android.content.Context
import android.net.Uri
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import de.hedwig.app.data.AppSettings
import de.hedwig.app.data.CloudApi
import de.hedwig.app.data.CloudApiException
import de.hedwig.app.data.SecretStore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

class UploadWorker(context: Context, parameters: WorkerParameters) :
    CoroutineWorker(context, parameters) {
    override suspend fun doWork(): Result = withContext(Dispatchers.IO) {
        val uri = inputData.getString("uri")?.let(Uri::parse) ?: return@withContext Result.failure()
        val space = inputData.getString("space_id") ?: return@withContext Result.failure()
        val path = inputData.getString("path") ?: return@withContext Result.failure()
        val key = inputData.getString("idempotency_key") ?: return@withContext Result.failure()
        val account = SecretStore(applicationContext).load() ?: return@withContext Result.failure(
            workDataOf("error" to "Nicht angemeldet"),
        )
        val pending = File(applicationContext.cacheDir, "pending/$key")
        try {
            if (!pending.exists()) {
                pending.parentFile?.mkdirs()
                val source = applicationContext.contentResolver.openInputStream(uri)
                    ?: return@withContext Result.failure(workDataOf("error" to "Datei ist nicht mehr zugänglich"))
                source.use { input -> pending.outputStream().use { output -> input.copyTo(output) } }
            }
            if (pending.length() > AppSettings(applicationContext).maximumUploadBytes) {
                return@withContext Result.failure(workDataOf("error" to "Datei überschreitet das eingestellte Größenlimit"))
            }
            setProgress(workDataOf("bytes" to pending.length(), "state" to "Übertragung"))
            val result = CloudApi(account).upload(space, path, pending, key)
            pending.delete()
            Result.success(workDataOf("file_id" to result.fileId, "path" to result.path, "conflict" to result.conflict))
        } catch (error: CloudApiException) {
            if (error.status >= 500 || error.status == 408 || error.status == 429) Result.retry()
            else Result.failure(workDataOf("error" to error.message))
        } catch (error: Exception) {
            if (runAttemptCount < 8) Result.retry()
            else Result.failure(workDataOf("error" to (error.message ?: "Upload fehlgeschlagen")))
        }
    }
}
