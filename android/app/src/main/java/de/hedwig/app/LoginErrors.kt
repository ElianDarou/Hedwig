package de.hedwig.app

import de.hedwig.app.data.CloudApiException
import java.net.ConnectException
import java.net.SocketTimeoutException
import java.net.UnknownHostException
import javax.net.ssl.SSLHandshakeException

internal fun loginErrorMessage(error: Throwable): String = when (error) {
    is CloudApiException -> when (error.status) {
        401 -> "Benutzername oder Passwort ist falsch."
        403 -> "Diese Anmeldung ist für das Konto nicht erlaubt."
        426 -> "Diese App-Version ist zu alt. Bitte aktualisiere Hedwig."
        else -> error.message.ifBlank { "Der Server hat die Anmeldung abgelehnt." }
    }
    is UnknownHostException ->
        "Der Servername wurde nicht gefunden. Prüfe die vollständige Adresse, zum Beispiel springfield.fritz.box."
    is SSLHandshakeException ->
        "Das Sicherheitszertifikat wird nicht vertraut. Installiere zuerst die CA deines Cloud-Servers."
    is SocketTimeoutException ->
        "Der Server antwortet nicht rechtzeitig. Prüfe WLAN, VPN und Serveradresse."
    is ConnectException ->
        "Der Server ist nicht erreichbar. Prüfe Netzwerk, Adresse und Port."
    is IllegalArgumentException ->
        error.message ?: "Die Serveradresse ist ungültig."
    else -> error.message ?: "Anmeldung fehlgeschlagen. Bitte versuche es erneut."
}
