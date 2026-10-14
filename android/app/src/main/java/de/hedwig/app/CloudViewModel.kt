package de.hedwig.app

import android.app.Application
import android.util.Log
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import de.hedwig.app.data.CloudAccount
import de.hedwig.app.data.CloudApi
import de.hedwig.app.data.CloudFile
import de.hedwig.app.data.SecretStore
import de.hedwig.app.data.Share
import de.hedwig.app.data.Space
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

data class CloudUiState(
    val account: CloudAccount? = null,
    val spaces: List<Space> = emptyList(),
    val selectedSpace: Space? = null,
    val files: List<CloudFile> = emptyList(),
    val trash: List<CloudFile> = emptyList(),
    val loading: Boolean = false,
    val error: String? = null,
    val query: String = "",
    val shares: List<Share> = emptyList(),
    val createdShareUrl: String? = null,
)

class CloudViewModel(application: Application) : AndroidViewModel(application) {
    private val secrets = SecretStore(application)
    private val mutableState = MutableStateFlow(CloudUiState(account = secrets.load()))
    val state = mutableState.asStateFlow()

    init { if (mutableState.value.account != null) refresh() }

    fun login(server: String, username: String, password: String, otp: String) = viewModelScope.launch {
        Log.i(LOGIN_LOG_TAG, "Login attempt started")
        update { it.copy(loading = true, error = null) }
        runCatching { CloudApi().login(server, username, password, otp.ifBlank { null }) }
            .onSuccess { account ->
                Log.i(LOGIN_LOG_TAG, "Login succeeded")
                secrets.save(account.server, account.session)
                update { it.copy(account = account, loading = false) }
                refresh()
            }
            .onFailure { error ->
                Log.w(LOGIN_LOG_TAG, "Login failed: ${error.javaClass.simpleName}")
                update { it.copy(loading = false, error = loginErrorMessage(error)) }
            }
    }

    fun refresh() = viewModelScope.launch {
        val account = mutableState.value.account ?: return@launch
        update { it.copy(loading = true, error = null) }
        runCatching {
            val api = CloudApi(account)
            val spaces = api.spaces()
            val selected = spaces.firstOrNull { it.id == mutableState.value.selectedSpace?.id } ?: spaces.firstOrNull()
            Triple(spaces, selected, selected?.let { api.files(it.id) }.orEmpty())
        }.onSuccess { (spaces, selected, files) ->
            update { it.copy(spaces = spaces, selectedSpace = selected, files = files, loading = false) }
        }.onFailure { error -> update { it.copy(loading = false, error = error.message ?: "Aktualisierung fehlgeschlagen") } }
    }

    fun selectSpace(space: Space) = viewModelScope.launch {
        val account = mutableState.value.account ?: return@launch
        update { it.copy(selectedSpace = space, loading = true, error = null) }
        runCatching { CloudApi(account).files(space.id) }
            .onSuccess { files -> update { it.copy(files = files, loading = false) } }
            .onFailure { error -> update { it.copy(loading = false, error = error.message) } }
    }

    fun selectTrashSpace(space: Space) = viewModelScope.launch {
        val account = mutableState.value.account ?: return@launch
        update { it.copy(selectedSpace = space, loading = true, error = null) }
        runCatching { CloudApi(account).files(space.id, true).filter { it.deletedAt != null } }
            .onSuccess { files -> update { it.copy(trash = files, loading = false) } }
            .onFailure { error -> update { it.copy(loading = false, error = error.message) } }
    }

    fun search(value: String) = update { it.copy(query = value) }

    fun loadTrash() = viewModelScope.launch {
        val account = mutableState.value.account ?: return@launch
        val space = mutableState.value.selectedSpace ?: return@launch
        update { it.copy(loading = true, error = null) }
        runCatching { CloudApi(account).files(space.id, true).filter { it.deletedAt != null } }
            .onSuccess { files -> update { it.copy(trash = files, loading = false) } }
            .onFailure { error -> update { it.copy(loading = false, error = error.message) } }
    }

    fun loadShares() = viewModelScope.launch {
        val account = mutableState.value.account ?: return@launch
        update { it.copy(loading = true, error = null) }
        runCatching { CloudApi(account).shares() }
            .onSuccess { shares -> update { it.copy(shares = shares, loading = false) } }
            .onFailure { error -> update { it.copy(loading = false, error = error.message) } }
    }

    fun createShare(spaceId: String, path: String, mode: String, password: String, expiryDays: Long?, maximumMb: Long?) = viewModelScope.launch {
        val account = mutableState.value.account ?: return@launch
        update { it.copy(loading = true, error = null, createdShareUrl = null) }
        val expiresAt = expiryDays?.let { java.time.Instant.now().plus(java.time.Duration.ofDays(it)).toString() }
        runCatching { CloudApi(account).createShare(spaceId, path, mode, password.ifBlank { null }, expiresAt, maximumMb?.times(1024 * 1024)) }
            .onSuccess { created ->
                val shares = CloudApi(account).shares()
                update { it.copy(shares = shares, loading = false, createdShareUrl = "${account.server}/${created.urlFragment}") }
            }
            .onFailure { error -> update { it.copy(loading = false, error = error.message) } }
    }

    fun revokeShare(shareId: String) = viewModelScope.launch {
        val account = mutableState.value.account ?: return@launch
        update { it.copy(loading = true, error = null) }
        runCatching { CloudApi(account).revokeShare(shareId); CloudApi(account).shares() }
            .onSuccess { shares -> update { it.copy(shares = shares, loading = false) } }
            .onFailure { error -> update { it.copy(loading = false, error = error.message) } }
    }

    fun clearCreatedShare() = update { it.copy(createdShareUrl = null) }

    fun delete(file: CloudFile) = mutateFile { it.deleteFile(file.id) }
    fun restore(file: CloudFile) = mutateFile { it.restoreFile(file.id) }

    private fun mutateFile(action: suspend (CloudApi) -> Unit) = viewModelScope.launch {
        val account = mutableState.value.account ?: return@launch
        update { it.copy(loading = true, error = null) }
        runCatching { action(CloudApi(account)) }
            .onSuccess { refresh() }
            .onFailure { error -> update { it.copy(loading = false, error = error.message) } }
    }

    fun clearError() = update { it.copy(error = null) }

    fun logout() = viewModelScope.launch {
        val account = mutableState.value.account ?: return@launch
        update { it.copy(loading = true, error = null) }
        runCatching { CloudApi(account).logout() }
            .onSuccess { clearLocalSession() }
            .onFailure { error ->
                if (error is de.hedwig.app.data.CloudApiException && error.status == 401) {
                    clearLocalSession()
                } else {
                    update {
                        it.copy(
                            loading = false,
                            error = "Abmelden am Server fehlgeschlagen: ${error.message ?: "Unbekannter Fehler"}",
                        )
                    }
                }
            }
    }

    private fun clearLocalSession() {
        secrets.clear()
        mutableState.value = CloudUiState()
    }

    private fun update(transform: (CloudUiState) -> CloudUiState) {
        mutableState.value = transform(mutableState.value)
    }

    private companion object {
        const val LOGIN_LOG_TAG = "HedwigLogin"
    }
}
