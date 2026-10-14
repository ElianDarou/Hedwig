package de.hedwig.app.documents

import android.database.Cursor
import android.database.MatrixCursor
import android.os.CancellationSignal
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract.Document
import android.provider.DocumentsContract.Root
import android.provider.DocumentsProvider
import android.util.Base64
import de.hedwig.app.R
import de.hedwig.app.data.CloudApi
import de.hedwig.app.data.CloudFile
import de.hedwig.app.data.SecretStore
import kotlinx.coroutines.runBlocking
import java.io.File
import java.io.FileNotFoundException
import java.net.URLConnection

class CloudDocumentsProvider : DocumentsProvider() {
    override fun onCreate() = true

    override fun queryRoots(projection: Array<out String>?): Cursor {
        val cursor = MatrixCursor(projection ?: DEFAULT_ROOT_PROJECTION)
        if (context?.let { SecretStore(it).load() } != null) {
            cursor.newRow().apply {
                add(Root.COLUMN_ROOT_ID, ROOT_ID)
                add(Root.COLUMN_DOCUMENT_ID, ROOT_DOCUMENT)
                add(Root.COLUMN_TITLE, context?.getString(R.string.documents_root) ?: "Hedwig")
                add(Root.COLUMN_SUMMARY, "Nur lesbarer Zugriff; Inhalte werden bei Bedarf geladen")
                add(Root.COLUMN_FLAGS, Root.FLAG_SUPPORTS_SEARCH)
                add(Root.COLUMN_MIME_TYPES, "*/*")
                add(Root.COLUMN_ICON, android.R.drawable.stat_sys_upload)
            }
        }
        return cursor
    }

    override fun queryDocument(documentId: String, projection: Array<out String>?): Cursor {
        val cursor = MatrixCursor(projection ?: DEFAULT_DOCUMENT_PROJECTION)
        if (documentId == ROOT_DOCUMENT) {
            addDirectory(cursor, ROOT_DOCUMENT, "Hedwig")
            return cursor
        }
        val account = account()
        val api = CloudApi(account)
        runBlocking {
            when {
                documentId.startsWith("space:") -> {
                    val id = documentId.removePrefix("space:")
                    val space = api.spaces().firstOrNull { it.id == id } ?: throw FileNotFoundException()
                    addDirectory(cursor, documentId, space.name)
                }
                documentId.startsWith("dir:") -> {
                    addDirectory(cursor, documentId, decodeDirectory(documentId).second.substringAfterLast('/'))
                }
                documentId.startsWith("file:") -> {
                    val (spaceId, file) = resolveFile(api, documentId)
                    addFile(cursor, spaceId, file)
                }
                else -> throw FileNotFoundException("Unbekanntes Cloud-Dokument")
            }
        }
        return cursor
    }

    override fun queryChildDocuments(
        parentDocumentId: String,
        projection: Array<out String>?,
        sortOrder: String?,
    ): Cursor {
        val cursor = MatrixCursor(projection ?: DEFAULT_DOCUMENT_PROJECTION)
        val api = CloudApi(account())
        runBlocking {
            if (parentDocumentId == ROOT_DOCUMENT) {
                api.spaces().forEach { addDirectory(cursor, "space:${it.id}", it.name) }
                return@runBlocking
            }
            val (spaceId, parentPath) = when {
                parentDocumentId.startsWith("space:") -> parentDocumentId.removePrefix("space:") to ""
                parentDocumentId.startsWith("dir:") -> decodeDirectory(parentDocumentId)
                else -> throw FileNotFoundException("Kein Cloud-Ordner")
            }
            val directories = linkedSetOf<String>()
            api.files(spaceId).forEach { file ->
                val relative = if (parentPath.isEmpty()) file.path else file.path.removePrefix("$parentPath/")
                if (relative == file.path && parentPath.isNotEmpty()) return@forEach
                if ('/' in relative) {
                    val childPath = listOf(parentPath, relative.substringBefore('/')).filter { it.isNotEmpty() }.joinToString("/")
                    if (directories.add(childPath)) addDirectory(cursor, directoryId(spaceId, childPath), childPath.substringAfterLast('/'))
                } else {
                    addFile(cursor, spaceId, file)
                }
            }
        }
        return cursor
    }

    override fun querySearchDocuments(
        rootId: String,
        query: String,
        projection: Array<out String>?,
    ): Cursor {
        val cursor = MatrixCursor(projection ?: DEFAULT_DOCUMENT_PROJECTION)
        val api = CloudApi(account())
        runBlocking {
            api.spaces().forEach { space ->
                api.files(space.id).asSequence().filter { it.path.contains(query, ignoreCase = true) }
                    .take(50).forEach { addFile(cursor, space.id, it) }
            }
        }
        return cursor
    }

    override fun openDocument(
        documentId: String,
        mode: String,
        signal: CancellationSignal?,
    ): ParcelFileDescriptor {
        if (mode != "r") throw FileNotFoundException("Cloud-Zugriff ist hier nur lesbar")
        val application = requireNotNull(context)
        val api = CloudApi(account())
        val (_, cloudFile) = runBlocking { resolveFile(api, documentId) }
        val offline = application.getSharedPreferences("offline-files", android.content.Context.MODE_PRIVATE)
            .getString(cloudFile.id, null)?.let(::File)?.takeIf(File::exists)
        val cached = offline ?: File(application.cacheDir, "cloud-downloads/${cloudFile.currentVersionId}/${cloudFile.path.substringAfterLast('/')}")
        if (!cached.exists()) {
            try {
                runBlocking { api.download(cloudFile.id, cached) }
            } catch (error: Exception) {
                throw FileNotFoundException("Inhalt ist offline nicht verfügbar: ${error.message}")
            }
        }
        return ParcelFileDescriptor.open(cached, ParcelFileDescriptor.MODE_READ_ONLY)
    }

    private fun account() = context?.let { SecretStore(it).load() }
        ?: throw FileNotFoundException("Nicht angemeldet")

    private suspend fun resolveFile(api: CloudApi, documentId: String): Pair<String, CloudFile> {
        val parts = documentId.split(':', limit = 3)
        if (parts.size != 3 || parts[0] != "file") throw FileNotFoundException()
        val file = api.files(parts[1]).firstOrNull { it.id == parts[2] } ?: throw FileNotFoundException()
        return parts[1] to file
    }

    private fun addDirectory(cursor: MatrixCursor, id: String, name: String) {
        cursor.newRow().apply {
            add(Document.COLUMN_DOCUMENT_ID, id)
            add(Document.COLUMN_DISPLAY_NAME, name)
            add(Document.COLUMN_MIME_TYPE, Document.MIME_TYPE_DIR)
            add(Document.COLUMN_FLAGS, 0)
        }
    }

    private fun addFile(cursor: MatrixCursor, spaceId: String, file: CloudFile) {
        cursor.newRow().apply {
            add(Document.COLUMN_DOCUMENT_ID, "file:$spaceId:${file.id}")
            add(Document.COLUMN_DISPLAY_NAME, file.path.substringAfterLast('/'))
            add(Document.COLUMN_MIME_TYPE, URLConnection.guessContentTypeFromName(file.path) ?: "application/octet-stream")
            add(Document.COLUMN_SIZE, file.size)
            add(Document.COLUMN_LAST_MODIFIED, 0)
            add(Document.COLUMN_FLAGS, 0)
        }
    }

    private fun directoryId(spaceId: String, path: String): String =
        "dir:$spaceId:${Base64.encodeToString(path.encodeToByteArray(), Base64.URL_SAFE or Base64.NO_WRAP)}"

    private fun decodeDirectory(documentId: String): Pair<String, String> {
        val parts = documentId.split(':', limit = 3)
        if (parts.size != 3) throw FileNotFoundException()
        return parts[1] to Base64.decode(parts[2], Base64.URL_SAFE or Base64.NO_WRAP).decodeToString()
    }

    companion object {
        private const val ROOT_ID = "hedwig"
        private const val ROOT_DOCUMENT = "root"
        private val DEFAULT_ROOT_PROJECTION = arrayOf(
            Root.COLUMN_ROOT_ID, Root.COLUMN_DOCUMENT_ID, Root.COLUMN_TITLE, Root.COLUMN_SUMMARY,
            Root.COLUMN_FLAGS, Root.COLUMN_MIME_TYPES, Root.COLUMN_ICON,
        )
        private val DEFAULT_DOCUMENT_PROJECTION = arrayOf(
            Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME, Document.COLUMN_MIME_TYPE,
            Document.COLUMN_FLAGS, Document.COLUMN_SIZE, Document.COLUMN_LAST_MODIFIED,
        )
    }
}
