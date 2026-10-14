package de.hedwig.app.work

import android.content.Context
import android.net.Uri
import androidx.documentfile.provider.DocumentFile
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import de.hedwig.app.data.AppSettings
import de.hedwig.app.data.CloudApi
import de.hedwig.app.data.CloudApiException
import de.hedwig.app.data.SecretStore
import de.hedwig.app.data.TreeSyncRecord
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.security.MessageDigest

class TreeSyncWorker(context: Context, parameters: WorkerParameters) : CoroutineWorker(context, parameters) {
    override suspend fun doWork(): Result = withContext(Dispatchers.IO) {
        val settings = AppSettings(applicationContext)
        val uri = settings.treeSyncUri?.let(Uri::parse) ?: return@withContext Result.success()
        val space = settings.treeSyncSpaceId ?: return@withContext Result.success()
        val root = DocumentFile.fromTreeUri(applicationContext, uri)
            ?: return@withContext Result.failure(workDataOf("error" to "Freigegebener Ordner ist nicht mehr erreichbar"))
        val account = SecretStore(applicationContext).load()
            ?: return@withContext Result.failure(workDataOf("error" to "Nicht angemeldet"))
        try {
            var uploaded = 0
            walk(root).forEach { (document, relative) ->
                val size = document.length()
                if (size > settings.maximumUploadBytes) return@forEach
                val signature = "$size:${document.lastModified()}"
                val key = document.uri.toString()
                val previous = settings.treeRecord(key)
                if (previous?.signature == signature) return@forEach
                val pending = File(applicationContext.cacheDir, "tree-sync/${digest(key)}")
                pending.parentFile?.mkdirs()
                applicationContext.contentResolver.openInputStream(document.uri).use { input ->
                    requireNotNull(input) { "Datei ${document.name} ist nicht mehr lesbar" }
                    pending.outputStream().use(input::copyTo)
                }
                val remotePath = listOf(settings.treeSyncPrefix.trim('/'), relative).filter(String::isNotBlank).joinToString("/")
                val commit = CloudApi(account).upload(
                    space, remotePath, pending, "tree-${digest("$key:$signature")}", previous?.fileId, previous?.versionId,
                )
                settings.setTreeRecord(key, TreeSyncRecord(commit.fileId, commit.versionId, signature))
                pending.delete()
                uploaded++
                setProgress(workDataOf("uploaded" to uploaded, "path" to relative))
            }
            Result.success(workDataOf("uploaded" to uploaded))
        } catch (error: CloudApiException) {
            if (error.status >= 500 || error.status == 408 || error.status == 429) Result.retry()
            else Result.failure(workDataOf("error" to error.message))
        } catch (error: SecurityException) {
            Result.failure(workDataOf("error" to "Android-Zugriff auf den freigegebenen Ordner wurde entzogen"))
        } catch (error: Exception) {
            if (runAttemptCount < 8) Result.retry() else Result.failure(workDataOf("error" to (error.message ?: "Ordnerabgleich fehlgeschlagen")))
        }
    }

    private fun walk(root: DocumentFile): Sequence<Pair<DocumentFile, String>> = sequence {
        suspend fun SequenceScope<Pair<DocumentFile, String>>.visit(directory: DocumentFile, prefix: String) {
            directory.listFiles().forEach { child ->
                val name = child.name?.replace('/', '_') ?: return@forEach
                val relative = listOf(prefix, name).filter(String::isNotEmpty).joinToString("/")
                if (child.isDirectory) visit(child, relative) else if (child.isFile) yield(child to relative)
            }
        }
        visit(root, "")
    }

    private fun digest(value: String): String = MessageDigest.getInstance("SHA-256")
        .digest(value.encodeToByteArray()).joinToString("") { "%02x".format(it) }
}
