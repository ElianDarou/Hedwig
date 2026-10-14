package de.hedwig.app

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextReplacement
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import java.util.concurrent.atomic.AtomicReference

class LoginFlowTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun submitsAllFieldsAndKeepsLoginErrorsVisible() {
        val submitted = AtomicReference<List<String>>()
        compose.setContent {
            var error by androidx.compose.runtime.remember {
                androidx.compose.runtime.mutableStateOf<String?>(null)
            }
            CloudTheme {
                LoginScreen(
                    loading = false,
                    error = error,
                    clearError = { error = null },
                    login = { server, username, password, otp ->
                        submitted.set(listOf(server, username, password, otp))
                        error = "Benutzername oder Passwort ist falsch."
                    },
                )
            }
        }

        compose.onNodeWithTag("login_server").performTextReplacement("https://springfield.fritz.box:8443")
        compose.onNodeWithTag("login_username").performTextReplacement("elianUser")
        compose.onNodeWithTag("login_password").performTextReplacement("not-the-password")
        compose.onNodeWithTag("login_otp").performTextReplacement("123456")
        compose.onNodeWithTag("login_submit")
            .performScrollTo()
            .assertIsDisplayed()
            .assertIsEnabled()
            .performClick()

        compose.waitUntil(5_000) { submitted.get() != null }
        assertEquals(
            listOf(
                "https://springfield.fritz.box:8443",
                "elianUser",
                "not-the-password",
                "123456",
            ),
            submitted.get(),
        )
        compose.onNodeWithTag("login_error").assertIsDisplayed()
        compose.onNodeWithText("Benutzername oder Passwort ist falsch.").assertIsDisplayed()
    }
}
