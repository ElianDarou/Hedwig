package de.hedwig.app

import androidx.test.platform.app.InstrumentationRegistry
import de.hedwig.app.data.CloudAccount
import de.hedwig.app.data.CloudApi
import de.hedwig.app.data.Session
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.net.HttpURLConnection
import java.net.URI

class SpringfieldDataFlowTest {
    @Test
    fun uploadsDownloadsVersionsSharesDeletesAndRestores() {
        runBlocking {
            val arguments = InstrumentationRegistry.getArguments()
            val password = arguments.getString("cloudPassword")
            val token = arguments.getString("cloudToken")
            assertTrue(
                "cloudPassword or cloudToken instrumentation argument is required",
                password != null || token != null,
            )
            val server = "https://springfield.fritz.box:8443"
            val account = token?.let {
                CloudAccount(server, Session(it, "", "benchmark", "", "", false))
            } ?: CloudApi().login(server, "elian", password!!)
            val spaceName = "Android-Emulator-Test-${System.currentTimeMillis()}"
            var spaceId: String? = null
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val source = File(context.cacheDir, "android-emulator-source.bin")
            val destination = File(context.cacheDir, "android-emulator-download.bin")

            try {
                spaceId = createSpace(account, spaceName)
                val initial = ByteArray(1_310_721) { index -> (index * 31).toByte() }
                source.writeBytes(initial)
                val api = CloudApi(account)
                val idempotencyKey = "android-emulator-${System.currentTimeMillis()}"

                val first = api.upload(spaceId, "Android/Testdatei.bin", source, idempotencyKey)
                assertFalse(first.conflict)
                val repeated = api.upload(spaceId, "Android/Testdatei.bin", source, idempotencyKey)
                assertEquals(first.fileId, repeated.fileId)
                assertEquals(first.versionId, repeated.versionId)

                val uploaded = api.files(spaceId).single { it.id == first.fileId }
                assertEquals("Android/Testdatei.bin", uploaded.path)
                assertEquals(initial.size.toLong(), uploaded.size)
                api.download(uploaded.id, destination)
                assertArrayEquals(initial, destination.readBytes())

                val firstHistory = api.versions(uploaded.id)
                assertEquals(1, firstHistory.size)
                assertTrue(firstHistory.single().current)

                val changed = initial.copyOf().also { bytes ->
                    for (index in 1_048_576 until 1_081_344) bytes[index] = (bytes[index] + 1).toByte()
                }
                source.writeBytes(changed)
                val second = api.upload(
                    spaceId,
                    uploaded.path,
                    source,
                    "$idempotencyKey-v2",
                    uploaded.id,
                    uploaded.currentVersionId,
                )
                assertEquals(uploaded.id, second.fileId)
                assertFalse(second.conflict)
                val history = api.versions(uploaded.id)
                assertEquals(2, history.size)
                assertEquals(1, history.count { it.current })
                api.download(uploaded.id, destination)
                assertArrayEquals(changed, destination.readBytes())

                val share = api.createShare(
                    spaceId,
                    uploaded.path,
                    "read",
                    "android-emulator-test",
                    java.time.Instant.now().plusSeconds(3_600).toString(),
                    null,
                )
                assertTrue(api.shares().any { it.id == share.share.id && it.passwordProtected })
                api.revokeShare(share.share.id)
                assertNotNull(api.shares().single { it.id == share.share.id }.revokedAt)

                api.deleteFile(uploaded.id)
                assertNotNull(api.files(spaceId, includeDeleted = true).single { it.id == uploaded.id }.deletedAt)
                api.restoreFile(uploaded.id)
                assertTrue(api.files(spaceId).any { it.id == uploaded.id && it.deletedAt == null })
            } finally {
                spaceId?.let { deleteSpace(account, it, spaceName) }
                if (token == null) runCatching { CloudApi(account).logout() }
                source.delete()
                destination.delete()
            }
        }
    }

    private fun createSpace(account: CloudAccount, name: String): String {
        val body = buildJsonObject { put("name", name) }.toString().encodeToByteArray()
        val response = request(account, "/v1/spaces", body)
        return Json.parseToJsonElement(response.decodeToString()).jsonObject
            .getValue("id").jsonPrimitive.content
    }

    private fun deleteSpace(account: CloudAccount, spaceId: String, name: String) {
        val body = buildJsonObject { put("name", name) }.toString().encodeToByteArray()
        request(account, "/v1/spaces/$spaceId/delete", body)
    }

    private fun request(account: CloudAccount, path: String, body: ByteArray): ByteArray {
        val connection = URI(account.server + path).toURL().openConnection() as HttpURLConnection
        return try {
            connection.requestMethod = "POST"
            connection.connectTimeout = 15_000
            connection.readTimeout = 30_000
            connection.setRequestProperty("Authorization", "Bearer ${account.session.token}")
            connection.setRequestProperty("Content-Type", "application/json")
            connection.setRequestProperty("Accept", "application/json")
            connection.setRequestProperty("X-Hedwig-Protocol", "1")
            connection.doOutput = true
            connection.setFixedLengthStreamingMode(body.size)
            connection.outputStream.use { it.write(body) }
            val status = connection.responseCode
            val response = (if (status in 200..299) connection.inputStream else connection.errorStream)
                ?.use { it.readBytes() }
                ?: byteArrayOf()
            check(status in 200..299) { "Springfield test request failed with HTTP $status" }
            response
        } finally {
            connection.disconnect()
        }
    }
}
