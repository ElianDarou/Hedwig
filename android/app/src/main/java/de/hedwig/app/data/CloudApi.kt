package de.hedwig.app.data

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.bouncycastle.crypto.digests.Blake3Digest
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.URI

class CloudApi(private val account: CloudAccount? = null) {
    private val json = Json { ignoreUnknownKeys = true }

    suspend fun login(server: String, username: String, password: String, otp: String? = null): CloudAccount =
        withContext(Dispatchers.IO) {
            val normalized = SecretStore.normalizeServer(server)
            require(URI(normalized).scheme == "https") { "Der Server muss HTTPS verwenden" }
            val body = buildJsonObject {
                put("username", username)
                put("password", password)
                put("device_name", "Android")
                if (!otp.isNullOrBlank()) put("otp", otp)
            }.toString().encodeToByteArray()
            val response = request(normalized, null, "POST", "/v1/auth/login", body, readTimeoutMs = 30_000)
            CloudAccount(normalized, json.decodeFromString(Session.serializer(), response.decodeToString()))
        }

    suspend fun spaces(): List<Space> = withContext(Dispatchers.IO) {
        json.decodeFromString(SpaceList.serializer(), authorized("GET", "/v1/spaces").decodeToString()).spaces
    }

    suspend fun logout() = withContext(Dispatchers.IO) {
        authorized("POST", "/v1/auth/logout")
    }

    suspend fun files(spaceId: String, includeDeleted: Boolean = false): List<CloudFile> =
        withContext(Dispatchers.IO) {
            val suffix = if (includeDeleted) "?include_deleted=true" else ""
            json.decodeFromString(
                FileList.serializer(),
                authorized("GET", "/v1/spaces/$spaceId/files$suffix").decodeToString(),
            ).files
        }

    suspend fun download(fileId: String, destination: File) = withContext(Dispatchers.IO) {
        val manifest = json.decodeFromString(
            FileManifest.serializer(),
            authorized("GET", "/v1/files/$fileId/manifest").decodeToString(),
        )
        destination.parentFile?.mkdirs()
        val partial = File(destination.parentFile, ".${destination.name}.partial")
        val active = requireNotNull(account) { "Nicht angemeldet" }
        val connection = openConnection(active.server, active.session.token, "GET", "/v1/files/$fileId/content", null)
        try {
            val status = connection.responseCode
            if (status !in 200..299) throw responseError(connection, status)
            var total = 0L
            connection.inputStream.buffered().use { input -> partial.outputStream().buffered().use { output ->
                manifest.blocks.forEach { block ->
                    val bytes = input.readUpTo(block.size)
                    check(bytes.size == block.size) { "Download ist unvollständig" }
                    check(blake3(bytes, bytes.size) == block.hash) { "Blockprüfung des Downloads ist fehlgeschlagen" }
                    output.write(bytes); total += bytes.size
                }
                check(input.read() == -1 && total == manifest.size) { "Downloadgröße stimmt nicht mit dem Manifest überein" }
            } }
        } catch (error: Exception) {
            partial.delete()
            throw error
        } finally { connection.disconnect() }
        if (destination.exists()) check(destination.delete()) { "Alte Offline-Datei konnte nicht ersetzt werden" }
        check(partial.renameTo(destination)) { "Download konnte nicht atomar gespeichert werden" }
    }

    suspend fun versions(fileId: String): List<FileVersion> = withContext(Dispatchers.IO) {
        json.decodeFromString(VersionList.serializer(), authorized("GET", "/v1/files/$fileId/versions").decodeToString()).versions
    }

    suspend fun deleteFile(fileId: String) = withContext(Dispatchers.IO) {
        authorized("DELETE", "/v1/files/$fileId", byteArrayOf())
    }

    suspend fun restoreFile(fileId: String) = withContext(Dispatchers.IO) {
        authorized("POST", "/v1/files/$fileId/restore", byteArrayOf())
    }

    suspend fun restoreVersion(fileId: String, versionId: String) = withContext(Dispatchers.IO) {
        authorized("POST", "/v1/files/$fileId/versions/$versionId/restore", byteArrayOf())
    }

    suspend fun createReadShare(spaceId: String, path: String): CreatedShare = withContext(Dispatchers.IO) {
        val body = buildJsonObject {
            put("space_id", spaceId); put("target_path", path); put("mode", "read")
        }.toString().encodeToByteArray()
        json.decodeFromString(CreatedShare.serializer(), authorized("POST", "/v1/shares", body).decodeToString())
    }

    suspend fun shares(): List<Share> = withContext(Dispatchers.IO) {
        json.decodeFromString(ShareList.serializer(), authorized("GET", "/v1/shares").decodeToString()).shares
    }

    suspend fun revokeShare(shareId: String) = withContext(Dispatchers.IO) {
        authorized("DELETE", "/v1/shares/$shareId", byteArrayOf())
    }

    suspend fun createShare(
        spaceId: String,
        path: String,
        mode: String,
        password: String?,
        expiresAt: String?,
        maxUploadSize: Long?,
    ): CreatedShare = withContext(Dispatchers.IO) {
        val body = buildJsonObject {
            put("space_id", spaceId)
            put("target_path", path)
            put("mode", mode)
            if (!password.isNullOrBlank()) put("password", password)
            if (expiresAt != null) put("expires_at", expiresAt)
            if (maxUploadSize != null) put("max_upload_size", maxUploadSize)
        }.toString().encodeToByteArray()
        json.decodeFromString(CreatedShare.serializer(), authorized("POST", "/v1/shares", body).decodeToString())
    }

    suspend fun upload(
        spaceId: String,
        path: String,
        source: File,
        idempotencyKey: String,
        fileId: String? = null,
        baseVersionId: String? = null,
    ): UploadCommit =
        withContext(Dispatchers.IO) {
            val chunks = chunk(source)
            val startBody = buildJsonObject {
                put("path", path)
                put("size", source.length())
                put("idempotency_key", idempotencyKey)
                if (fileId != null) put("file_id", fileId)
                if (baseVersionId != null) put("base_version_id", baseVersionId)
                put("blocks", buildJsonArray {
                    chunks.forEach { chunk -> add(buildJsonObject {
                        put("hash", chunk.hash)
                        put("size", chunk.size)
                    }) }
                })
            }.toString().encodeToByteArray()
            val start = json.decodeFromString(
                UploadStart.serializer(),
                authorized("POST", "/v1/spaces/$spaceId/uploads", startBody).decodeToString(),
            )
            val missing = start.missingBlocks.toHashSet()
            source.inputStream().buffered().use { input ->
                chunks.forEach { chunk ->
                    val bytes = input.readUpTo(chunk.size)
                    check(bytes.size == chunk.size) { "Quelldatei wurde während des Uploads geändert" }
                    check(blake3(bytes, bytes.size) == chunk.hash) {
                        "Quelldatei wurde während des Uploads geändert"
                    }
                    if (chunk.hash in missing) {
                        authorized("PUT", "/v1/uploads/${start.uploadId}/blocks/${chunk.hash}", bytes)
                    }
                }
                check(input.read() == -1) { "Quelldatei wurde während des Uploads geändert" }
            }
            json.decodeFromString(
                UploadCommit.serializer(),
                authorized("POST", "/v1/uploads/${start.uploadId}/commit", byteArrayOf()).decodeToString(),
            )
        }

    private fun authorized(method: String, path: String, body: ByteArray? = null): ByteArray {
        val active = requireNotNull(account) { "Nicht angemeldet" }
        return request(active.server, active.session.token, method, path, body)
    }

    private fun request(
        server: String,
        token: String?,
        method: String,
        path: String,
        body: ByteArray?,
        readTimeoutMs: Int = 120_000,
    ): ByteArray {
        val connection = openConnection(server, token, method, path, body, readTimeoutMs)
        val status = connection.responseCode
        val stream = if (status in 200..299) connection.inputStream else connection.errorStream
        val response = stream?.use { it.readBytes() } ?: byteArrayOf()
        connection.disconnect()
        if (status !in 200..299) throw parseError(status, response)
        return response
    }

    private fun openConnection(
        server: String,
        token: String?,
        method: String,
        path: String,
        body: ByteArray?,
        readTimeoutMs: Int = 120_000,
    ): HttpURLConnection {
        val connection = URI(server + path).toURL().openConnection() as HttpURLConnection
        connection.requestMethod = method
        connection.connectTimeout = 15_000
        connection.readTimeout = readTimeoutMs
        connection.setRequestProperty("Accept", "application/json")
        connection.setRequestProperty("X-Hedwig-Protocol", "1")
        if (token != null) connection.setRequestProperty("Authorization", "Bearer $token")
        if (body != null) {
            connection.doOutput = true
            connection.setRequestProperty("Content-Type", "application/json")
            connection.setFixedLengthStreamingMode(body.size)
            connection.outputStream.use { it.write(body) }
        }
        return connection
    }

    private fun responseError(connection: HttpURLConnection, status: Int): CloudApiException {
        val response = connection.errorStream?.use { it.readBytes() } ?: byteArrayOf()
        return parseError(status, response)
    }

    private fun parseError(status: Int, response: ByteArray): CloudApiException {
        val message = runCatching {
            json.parseToJsonElement(response.decodeToString()).jsonObject["error"]
                ?.jsonObject?.get("message")?.jsonPrimitive?.contentOrNull
        }.getOrNull()
        return CloudApiException(status, message ?: "Serverfehler $status")
    }

    private fun chunk(file: File): List<BlockDescriptor> {
        val chunks = mutableListOf<BlockDescriptor>()
        file.inputStream().buffered().use { input ->
            val buffer = ByteArray(MAX_BLOCK_SIZE)
            while (true) {
                var used = 0
                while (used < buffer.size) {
                    val read = input.read(buffer, used, buffer.size - used)
                    if (read < 0) break
                    used += read
                }
                if (used == 0) break
                chunks += BlockDescriptor(blake3(buffer, used), used)
            }
        }
        return chunks
    }

    private fun blake3(bytes: ByteArray, length: Int): String {
        val digest = Blake3Digest(256)
        digest.update(bytes, 0, length)
        val output = ByteArray(32)
        digest.doFinal(output, 0)
        return output.joinToString("") { "%02x".format(it) }
    }

    companion object { const val MAX_BLOCK_SIZE = 1024 * 1024 }
}

private fun InputStream.readUpTo(size: Int): ByteArray {
    val bytes = ByteArray(size)
    var offset = 0
    while (offset < size) {
        val read = read(bytes, offset, size - offset)
        if (read < 0) break
        if (read == 0) continue
        offset += read
    }
    return if (offset == size) bytes else bytes.copyOf(offset)
}

class CloudApiException(val status: Int, override val message: String) : IOException(message)
