package de.hedwig.app

import androidx.test.platform.app.InstrumentationRegistry
import de.hedwig.app.data.CloudAccount
import de.hedwig.app.data.CloudApi
import de.hedwig.app.data.CloudApiException
import de.hedwig.app.data.Session
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class SpringfieldApiTest {
    @Test
    fun rejectsWrongCredentialsAndLoadsSpacesAfterRealLogin() {
        runBlocking {
            val arguments = InstrumentationRegistry.getArguments()
            val password = arguments.getString("cloudPassword")
            val token = arguments.getString("cloudToken")
            assertTrue(
                "cloudPassword or cloudToken instrumentation argument is required",
                password != null || token != null,
            )
            val server = "https://springfield.fritz.box:8443"

            val rejected = runCatching {
                CloudApi().login(server, "elianUser", "not-the-password")
            }.exceptionOrNull()
            assertTrue(rejected is CloudApiException)
            assertEquals(401, (rejected as CloudApiException).status)

            val account = token?.let {
                CloudAccount(server, Session(it, "", "benchmark", "", "", false))
            } ?: CloudApi().login(server, "elian", password!!)
            try {
                val spaces = CloudApi(account).spaces()
                assertTrue("The Springfield account must expose at least one space", spaces.isNotEmpty())
            } finally {
                if (token == null) CloudApi(account).logout()
            }
        }
    }
}
