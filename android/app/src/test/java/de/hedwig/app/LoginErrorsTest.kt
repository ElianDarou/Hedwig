package de.hedwig.app

import de.hedwig.app.data.CloudApiException
import java.net.SocketTimeoutException
import java.net.UnknownHostException
import javax.net.ssl.SSLHandshakeException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class LoginErrorsTest {
    @Test
    fun unauthorizedLoginExplainsTheCredentialProblem() {
        assertEquals(
            "Benutzername oder Passwort ist falsch.",
            loginErrorMessage(CloudApiException(401, "invalid credentials")),
        )
    }

    @Test
    fun networkFailuresGiveAnActionableNextStep() {
        assertTrue(loginErrorMessage(UnknownHostException()).contains("vollständige Adresse"))
        assertTrue(loginErrorMessage(SocketTimeoutException()).contains("WLAN"))
        assertTrue(loginErrorMessage(SSLHandshakeException("certificate rejected")).contains("CA"))
    }
}
