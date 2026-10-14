package de.hedwig.app

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextReplacement
import androidx.test.platform.app.InstrumentationRegistry
import de.hedwig.app.data.CloudApi
import de.hedwig.app.data.SecretStore
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertNotNull
import org.junit.Rule
import org.junit.Test

class SpringfieldLoginUiTest {
    @get:Rule
    val compose = createAndroidComposeRule<MainActivity>()

    @Test
    fun rejectsWrongPasswordLogsInAndOpensCentralSections() {
        val password = InstrumentationRegistry.getArguments().getString("cloudPassword")
        assertNotNull("cloudPassword instrumentation argument is required", password)

        replace("login_server", "https://springfield.fritz.box:8443")
        replace("login_username", "elian")
        replace("login_password", "not-the-password")
        submit()

        waitForTag("login_error", 30_000)
        compose.onNodeWithText("Benutzername oder Passwort ist falsch.").assertIsDisplayed()

        replace("login_password", password!!)
        submit()

        waitForText("Offline", 30_000)
        compose.onNodeWithText("Offline").performClick()
        waitForText("Noch keine Dateien offline gespeichert.", 10_000)

        compose.onNodeWithText("Geteilt").performClick()
        waitForText("Freigabelinks", 20_000)

        compose.onNodeWithText("Papierkorb").performClick()
        compose.waitForIdle()

        compose.onNodeWithText("Einstellungen").performClick()
        waitForText("Übertragungen", 10_000)

        val storedAccount = SecretStore(compose.activity).load()
        assertNotNull("The successful UI login must persist the device session", storedAccount)
        runBlocking { CloudApi(storedAccount!!).logout() }
        SecretStore(compose.activity).clear()
    }

    private fun replace(tag: String, value: String) {
        compose.onNodeWithTag(tag).performScrollTo().performTextReplacement(value)
    }

    private fun submit() {
        compose.onNodeWithTag("login_submit")
            .performScrollTo()
            .assertIsDisplayed()
            .performClick()
    }

    private fun waitForTag(tag: String, timeoutMillis: Long) {
        compose.waitUntil(timeoutMillis) {
            runCatching { compose.onNodeWithTag(tag).assertIsDisplayed() }.isSuccess
        }
    }

    private fun waitForText(text: String, timeoutMillis: Long) {
        compose.waitUntil(timeoutMillis) {
            runCatching { compose.onNodeWithText(text).assertIsDisplayed() }.isSuccess
        }
    }
}
