package de.hedwig.app

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.webkit.MimeTypeMap
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Logout
import androidx.compose.material.icons.automirrored.outlined.OpenInNew
import androidx.compose.material.icons.outlined.Cloud
import androidx.compose.material.icons.outlined.CloudDownload
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.History
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.outlined.Share
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.Link
import androidx.compose.material.icons.outlined.UploadFile
import androidx.compose.material.icons.outlined.Visibility
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material3.Button
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.core.content.FileProvider
import androidx.core.content.edit
import androidx.core.net.toUri
import de.hedwig.app.data.AppSettings
import de.hedwig.app.data.CloudAccount
import de.hedwig.app.data.CloudApi
import de.hedwig.app.data.CloudFile
import de.hedwig.app.data.FileVersion
import de.hedwig.app.data.Space
import de.hedwig.app.work.TransferScheduler
import kotlinx.coroutines.launch
import java.io.File

class MainActivity : ComponentActivity() {
    private val viewModel by viewModels<CloudViewModel>()
    private var incomingUris by mutableStateOf<List<Uri>>(emptyList())

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        consumeShareIntent(intent)
        setContent { CloudTheme { CloudApp(viewModel, incomingUris) { incomingUris = emptyList() } } }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        consumeShareIntent(intent)
    }

    private fun consumeShareIntent(intent: Intent?) {
        incomingUris = when (intent?.action) {
            Intent.ACTION_SEND -> listOfNotNull(intent.sharedUri())
            Intent.ACTION_SEND_MULTIPLE -> intent.sharedUris()
            else -> emptyList()
        }
    }

    @Suppress("DEPRECATION")
    private fun Intent.sharedUri(): Uri? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
        } else {
            getParcelableExtra(Intent.EXTRA_STREAM)
        }

    @Suppress("DEPRECATION")
    private fun Intent.sharedUris(): List<Uri> =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java).orEmpty()
        } else {
            getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM).orEmpty()
        }
}

private enum class Section { Files, Offline, Shares, Trash, Settings }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun CloudApp(viewModel: CloudViewModel, incoming: List<Uri>, consumed: () -> Unit) {
    val state by viewModel.state.collectAsState()
    var section by remember { mutableStateOf(Section.Files) }
    val snackbar = remember { SnackbarHostState() }
    val context = androidx.compose.ui.platform.LocalContext.current
    var pendingLogin by remember { mutableStateOf<List<String>?>(null) }
    val localNetwork = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        pendingLogin?.let { (server, username, password, otp) -> viewModel.login(server, username, password, otp) }
        pendingLogin = null
        if (state.account != null) viewModel.refresh()
    }
    val needsLocalNetwork = Build.VERSION.SDK_INT >= 37 &&
        context.checkSelfPermission(Manifest.permission.ACCESS_LOCAL_NETWORK) != PackageManager.PERMISSION_GRANTED
    LaunchedEffect(Unit) {
        if (state.account != null && needsLocalNetwork) localNetwork.launch(Manifest.permission.ACCESS_LOCAL_NETWORK)
    }
    LaunchedEffect(state.error, state.account) {
        if (state.account != null) {
            state.error?.let { snackbar.showSnackbar(it); viewModel.clearError() }
        }
    }
    if (state.account == null) {
        LoginScreen(state.loading, state.error, viewModel::clearError) { server, username, password, otp ->
            if (needsLocalNetwork) {
                pendingLogin = listOf(server, username, password, otp)
                localNetwork.launch(Manifest.permission.ACCESS_LOCAL_NETWORK)
            } else {
                viewModel.login(server, username, password, otp)
            }
        }
        return
    }
    Scaffold(
        snackbarHost = { SnackbarHost(snackbar) },
        topBar = {
            TopAppBar(
                title = { Text(state.selectedSpace?.name ?: "Hedwig", fontWeight = FontWeight.SemiBold) },
                actions = {
                    IconButton(onClick = viewModel::refresh) { Icon(Icons.Outlined.Refresh, "Jetzt abgleichen") }
                    IconButton(onClick = viewModel::logout) { Icon(Icons.AutoMirrored.Outlined.Logout, "Abmelden") }
                },
            )
        },
        bottomBar = {
            NavigationBar {
                NavigationBarItem(section == Section.Files, { section = Section.Files }, { Icon(Icons.Outlined.Folder, null) }, label = { Text("Dateien") })
                NavigationBarItem(section == Section.Offline, { section = Section.Offline }, { Icon(Icons.Outlined.CloudDownload, null) }, label = { Text("Offline") })
                NavigationBarItem(section == Section.Shares, { section = Section.Shares; viewModel.loadShares() }, { Icon(Icons.Outlined.Link, null) }, label = { Text("Geteilt") })
                NavigationBarItem(section == Section.Trash, { section = Section.Trash; viewModel.loadTrash() }, { Icon(Icons.Outlined.Delete, null) }, label = { Text("Papierkorb") })
                NavigationBarItem(section == Section.Settings, { section = Section.Settings }, { Icon(Icons.Outlined.Settings, null) }, label = { Text("Einstellungen") })
            }
        },
    ) { padding ->
        Box(Modifier.padding(padding).fillMaxSize()) {
            when (section) {
                Section.Files -> FilesScreen(state, incoming, consumed, viewModel::selectSpace, viewModel::search, viewModel::delete, viewModel::refresh)
                Section.Offline -> OfflineScreen()
                Section.Shares -> SharesScreen(state, viewModel::createShare, viewModel::revokeShare, viewModel::clearCreatedShare)
                Section.Trash -> TrashScreen(state, viewModel::selectTrashSpace, viewModel::restore)
                Section.Settings -> SettingsScreen(state.spaces)
            }
            if (state.loading) CircularProgressIndicator(Modifier.align(Alignment.Center))
        }
    }
}

@Composable
private fun SharesScreen(
    state: CloudUiState,
    create: (String, String, String, String, Long?, Long?) -> Unit,
    revoke: (String) -> Unit,
    clearCreated: () -> Unit,
) {
    val context = androidx.compose.ui.platform.LocalContext.current
    var showCreate by remember { mutableStateOf(false) }
    Column(Modifier.fillMaxSize().padding(16.dp)) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
            Text("Freigabelinks", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            Button({ showCreate = true }) { Text("Link erstellen") }
        }
        Spacer(Modifier.height(12.dp))
        if (state.shares.isEmpty() && !state.loading) EmptyMessage("Noch keine Freigaben.")
        else LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) { items(state.shares, key = { it.id }) { share ->
            Card(Modifier.fillMaxWidth(), colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow)) {
                Row(Modifier.fillMaxWidth().padding(14.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(share.targetPath, fontWeight = FontWeight.Medium)
                        Text((if (share.mode == "upload") "Dateien annehmen" else "Lesen/Download") + (if (share.passwordProtected) " · Passwort" else "") + (share.expiresAt?.let { " · bis ${it.take(10)}" } ?: ""), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    if (share.revokedAt == null) TextButton({ revoke(share.id) }) { Text("Widerrufen") } else Text("Widerrufen", color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        } }
    }
    if (showCreate) ShareCreateDialog(state.spaces, { showCreate = false }, create)
    state.createdShareUrl?.let { url -> AlertDialog(
        onDismissRequest = clearCreated,
        title = { Text("Freigabelink erstellt") },
        text = { Text("Der geheime Link wird aus Sicherheitsgründen nur jetzt vollständig gezeigt.\n\n$url") },
        confirmButton = { TextButton({
            val intent = Intent(Intent.ACTION_SEND).apply { type = "text/plain"; putExtra(Intent.EXTRA_TEXT, url) }
            context.startActivity(Intent.createChooser(intent, "Freigabelink senden")); clearCreated()
        }) { Text("Teilen") } },
        dismissButton = { TextButton(clearCreated) { Text("Schließen") } },
    ) }
}

@Composable
private fun ShareCreateDialog(spaces: List<Space>, close: () -> Unit, create: (String, String, String, String, Long?, Long?) -> Unit) {
    var space by remember { mutableStateOf(spaces.firstOrNull()?.id.orEmpty()) }
    var path by remember { mutableStateOf("") }; var mode by remember { mutableStateOf("read") }
    var password by remember { mutableStateOf("") }; var days by remember { mutableStateOf("") }; var maximum by remember { mutableStateOf("") }
    AlertDialog(onDismissRequest = close, title = { Text("Freigabelink erstellen") }, text = { Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) { spaces.forEach { item -> if (item.id == space) Button({ space = item.id }) { Text(item.name) } else OutlinedButton({ space = item.id }) { Text(item.name) } } }
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) { if (mode == "read") Button({ mode = "read" }) { Text("Lesen") } else OutlinedButton({ mode = "read" }) { Text("Lesen") }; if (mode == "upload") Button({ mode = "upload" }) { Text("Dateien annehmen") } else OutlinedButton({ mode = "upload" }) { Text("Dateien annehmen") } }
        OutlinedTextField(path, { path = it }, Modifier.fillMaxWidth(), label = { Text(if (mode == "upload") "Zielordner" else "Datei oder Ordner") }, singleLine = true)
        OutlinedTextField(password, { password = it }, Modifier.fillMaxWidth(), label = { Text("Passwort (optional)") }, singleLine = true)
        OutlinedTextField(days, { days = it.filter(Char::isDigit) }, Modifier.fillMaxWidth(), label = { Text("Ablauf in Tagen (optional)") }, singleLine = true)
        if (mode == "upload") OutlinedTextField(maximum, { maximum = it.filter(Char::isDigit) }, Modifier.fillMaxWidth(), label = { Text("Maximale Uploadgröße in MB") }, singleLine = true)
    } }, confirmButton = { TextButton({ create(space,path,mode,password,days.toLongOrNull(),maximum.toLongOrNull()); close() }, enabled = space.isNotBlank() && path.isNotBlank()) { Text("Erstellen") } }, dismissButton = { TextButton(close) { Text("Abbrechen") } })
}

@Composable
internal fun LoginScreen(
    loading: Boolean,
    error: String?,
    clearError: () -> Unit,
    login: (String, String, String, String) -> Unit,
) {
    var server by remember { mutableStateOf("") }
    var username by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var otp by remember { mutableStateOf("") }
    var passwordVisible by remember { mutableStateOf(false) }
    val focusManager = LocalFocusManager.current
    val canSubmit = server.isNotBlank() && username.isNotBlank() && password.isNotBlank()
    val submit = {
        if (!loading && canSubmit) {
            focusManager.clearFocus()
            login(server, username, password, otp)
        }
    }
    val fieldColors = OutlinedTextFieldDefaults.colors(
        focusedTextColor = MaterialTheme.colorScheme.onSurface,
        unfocusedTextColor = MaterialTheme.colorScheme.onSurface,
        focusedContainerColor = MaterialTheme.colorScheme.surface,
        unfocusedContainerColor = MaterialTheme.colorScheme.surface,
        cursorColor = MaterialTheme.colorScheme.primary,
        focusedBorderColor = MaterialTheme.colorScheme.primary,
        unfocusedBorderColor = if (isSystemInDarkTheme()) Color(0xFF5C544D) else Color(0xFFBFB6AD),
        focusedPlaceholderColor = MaterialTheme.colorScheme.onSurfaceVariant,
        unfocusedPlaceholderColor = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    Surface(color = MaterialTheme.colorScheme.background) {
        LazyColumn(
            Modifier.fillMaxSize().imePadding(),
            contentPadding = PaddingValues(horizontal = 20.dp, vertical = 28.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center,
        ) {
            item {
                Surface(
                    modifier = Modifier.fillMaxWidth().widthIn(max = 440.dp),
                    shape = RoundedCornerShape(12.dp),
                    color = MaterialTheme.colorScheme.surface,
                    border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
                ) {
                    Column(Modifier.padding(horizontal = 22.dp, vertical = 24.dp)) {
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                            Icon(
                                Icons.Outlined.Cloud,
                                contentDescription = null,
                                modifier = Modifier.size(24.dp),
                                tint = MaterialTheme.colorScheme.primary,
                            )
                            Text("Hedwig", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
                        }
                        Spacer(Modifier.height(32.dp))
                        Text("Anmelden", style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.SemiBold)
                        Spacer(Modifier.height(6.dp))
                        Text(
                            "Melde dich an deinem eigenen Cloud-Server an.",
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        if (error != null) {
                            Spacer(Modifier.height(18.dp))
                            Surface(
                                modifier = Modifier.testTag("login_error"),
                                color = MaterialTheme.colorScheme.errorContainer,
                                contentColor = MaterialTheme.colorScheme.onErrorContainer,
                                shape = RoundedCornerShape(8.dp),
                                border = BorderStroke(1.dp, MaterialTheme.colorScheme.error.copy(alpha = 0.35f)),
                            ) {
                                Row(
                                    Modifier.fillMaxWidth().padding(start = 12.dp, top = 8.dp, bottom = 8.dp, end = 4.dp),
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    Text(error, Modifier.weight(1f), style = MaterialTheme.typography.bodySmall)
                                    TextButton(onClick = clearError) { Text("Schließen") }
                                }
                            }
                        }
                        Spacer(Modifier.height(22.dp))
                        Text("Serveradresse", style = MaterialTheme.typography.labelLarge, fontWeight = FontWeight.SemiBold)
                        Spacer(Modifier.height(6.dp))
                        OutlinedTextField(
                            value = server,
                            onValueChange = { server = it },
                            modifier = Modifier.fillMaxWidth().testTag("login_server"),
                            placeholder = { Text("https://cloud.example.de") },
                            singleLine = true,
                            readOnly = loading,
                            shape = RoundedCornerShape(8.dp),
                            colors = fieldColors,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Next),
                        )
                        Spacer(Modifier.height(14.dp))
                        Text("Benutzername", style = MaterialTheme.typography.labelLarge, fontWeight = FontWeight.SemiBold)
                        Spacer(Modifier.height(6.dp))
                        OutlinedTextField(
                            value = username,
                            onValueChange = { username = it },
                            modifier = Modifier.fillMaxWidth().testTag("login_username"),
                            singleLine = true,
                            readOnly = loading,
                            shape = RoundedCornerShape(8.dp),
                            colors = fieldColors,
                            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Next),
                        )
                        Spacer(Modifier.height(14.dp))
                        Text("Passwort", style = MaterialTheme.typography.labelLarge, fontWeight = FontWeight.SemiBold)
                        Spacer(Modifier.height(6.dp))
                        OutlinedTextField(
                            value = password,
                            onValueChange = { password = it },
                            modifier = Modifier.fillMaxWidth().testTag("login_password"),
                            visualTransformation = if (passwordVisible) VisualTransformation.None else PasswordVisualTransformation(),
                            trailingIcon = {
                                IconButton(onClick = { passwordVisible = !passwordVisible }) {
                                    Icon(
                                        if (passwordVisible) Icons.Outlined.VisibilityOff else Icons.Outlined.Visibility,
                                        if (passwordVisible) "Passwort verbergen" else "Passwort anzeigen",
                                    )
                                }
                            },
                            singleLine = true,
                            readOnly = loading,
                            shape = RoundedCornerShape(8.dp),
                            colors = fieldColors,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Next),
                        )
                        Spacer(Modifier.height(14.dp))
                        Text("2FA-Code", style = MaterialTheme.typography.labelLarge, fontWeight = FontWeight.SemiBold)
                        Spacer(Modifier.height(6.dp))
                        OutlinedTextField(
                            value = otp,
                            onValueChange = { otp = it.filter(Char::isDigit).take(6) },
                            modifier = Modifier.fillMaxWidth().testTag("login_otp"),
                            placeholder = { Text("Optional") },
                            singleLine = true,
                            readOnly = loading,
                            shape = RoundedCornerShape(8.dp),
                            colors = fieldColors,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword, imeAction = ImeAction.Done),
                            keyboardActions = KeyboardActions(onDone = { submit() }),
                        )
                        Spacer(Modifier.height(22.dp))
                        Button(
                            onClick = submit,
                            modifier = Modifier.fillMaxWidth().heightIn(min = 52.dp).testTag("login_submit"),
                            enabled = !loading && canSubmit,
                            shape = RoundedCornerShape(8.dp),
                        ) {
                            if (loading) {
                                CircularProgressIndicator(
                                    Modifier.size(20.dp),
                                    color = MaterialTheme.colorScheme.onPrimary,
                                    strokeWidth = 2.dp,
                                )
                                Spacer(Modifier.size(10.dp))
                            }
                            Text(if (loading) "Anmeldung wird geprüft …" else "Anmelden")
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun FilesScreen(
    state: CloudUiState,
    incoming: List<Uri>,
    consumed: () -> Unit,
    selectSpace: (Space) -> Unit,
    search: (String) -> Unit,
    delete: (CloudFile) -> Unit,
    refresh: () -> Unit,
) {
    val context = androidx.compose.ui.platform.LocalContext.current
    var currentPath by remember(state.selectedSpace?.id) { mutableStateOf("") }
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        state.selectedSpace?.let { space -> uris.forEach { uri ->
            val name = queryName(context, uri)
            TransferScheduler.upload(context, uri, space.id, "Android/$name")
        } }
    }
    LaunchedEffect(incoming, state.selectedSpace) {
        val space = state.selectedSpace ?: return@LaunchedEffect
        incoming.forEach { uri -> TransferScheduler.upload(context, uri, space.id, "Android/${queryName(context, uri)}") }
        if (incoming.isNotEmpty()) consumed()
    }
    Column(Modifier.fillMaxSize().padding(horizontal = 16.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            state.spaces.forEach { space ->
                if (space.id == state.selectedSpace?.id) Button({ selectSpace(space) }) { Text(space.name) }
                else OutlinedButton({ selectSpace(space) }) { Text(space.name) }
            }
        }
        Spacer(Modifier.height(12.dp))
        OutlinedTextField(
            state.query, search, Modifier.fillMaxWidth(),
            leadingIcon = { Icon(Icons.Outlined.Search, null) }, label = { Text("Name oder Pfad suchen") }, singleLine = true,
        )
        Row(Modifier.fillMaxWidth().padding(vertical = 12.dp), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
            Text("${state.files.size} Dateien", color = MaterialTheme.colorScheme.onSurfaceVariant)
            TextButton({ picker.launch(arrayOf("*/*")) }) { Icon(Icons.Outlined.UploadFile, null); Text(" Hochladen") }
        }
        if (currentPath.isNotEmpty() && state.query.isBlank()) TextButton({ currentPath = currentPath.substringBeforeLast('/', "") }) { Text("‹ ${currentPath.substringAfterLast('/')}") }
        val prefix = currentPath.takeIf { it.isNotEmpty() }?.plus("/").orEmpty()
        val children = state.files.filter { it.path.startsWith(prefix) }
        val folderNames = children.mapNotNull { file -> file.path.removePrefix(prefix).takeIf { it.contains('/') }?.substringBefore('/') }.distinct().sorted()
        val visible = if (state.query.isNotBlank()) state.files.filter { it.path.contains(state.query, ignoreCase = true) }
        else children.filter { !it.path.removePrefix(prefix).contains('/') }
        if (folderNames.isEmpty() && visible.isEmpty() && !state.loading) EmptyMessage(if (state.query.isBlank()) "Dieser Ordner ist leer." else "Keine passende Datei.")
        LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            if (state.query.isBlank()) items(folderNames, key = { "folder:$it" }) { folder ->
                val fullPath = listOf(currentPath, folder).filter(String::isNotEmpty).joinToString("/")
                Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow), shape = RoundedCornerShape(14.dp)) {
                    Row(Modifier.fillMaxWidth().padding(14.dp), verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.Outlined.Folder, null, tint = MaterialTheme.colorScheme.primary)
                        TextButton({ currentPath = fullPath }, Modifier.weight(1f)) { Text(folder, Modifier.fillMaxWidth()) }
                        TextButton({ state.selectedSpace?.let { space -> state.files.filter { it.path.startsWith("$fullPath/") }.forEach { TransferScheduler.keepOffline(context, space.id, it.id, it.path) } } }) { Text("Offline") }
                    }
                }
            }
            items(visible, key = CloudFile::id) { file -> state.account?.let { FileCard(it, state.selectedSpace, file, delete, refresh) } }
        }
    }
}

@Composable
private fun FileCard(account: CloudAccount, space: Space?, file: CloudFile, delete: (CloudFile) -> Unit, refresh: () -> Unit) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val scope = rememberCoroutineScope()
    var menu by remember { mutableStateOf(false) }
    var versions by remember { mutableStateOf<List<FileVersion>?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    fun open(send: Boolean) = scope.launch {
        runCatching {
            val destination = File(context.cacheDir, "cloud-downloads/${file.currentVersionId}/${safeFileName(file.path)}")
            CloudApi(account).download(file.id, destination)
            val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", destination)
            val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(destination.extension.lowercase()) ?: "application/octet-stream"
            val intent = if (send) Intent(Intent.ACTION_SEND).apply { type = mime; putExtra(Intent.EXTRA_STREAM, uri) }
            else Intent(Intent.ACTION_VIEW).apply { setDataAndType(uri, mime) }
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            context.startActivity(Intent.createChooser(intent, if (send) "Datei weitergeben" else "Datei öffnen"))
        }.onFailure { error = it.message ?: "Datei konnte nicht geöffnet werden" }
    }
    Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow), shape = RoundedCornerShape(14.dp)) {
        Row(Modifier.fillMaxWidth().padding(14.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Outlined.Folder, null, tint = MaterialTheme.colorScheme.primary)
            Column(Modifier.weight(1f).padding(horizontal = 12.dp)) {
                Text(file.path.substringAfterLast('/'), fontWeight = FontWeight.Medium)
                Text(file.path.substringBeforeLast('/', "Cloud") + " · " + formatBytes(file.size), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            IconButton(onClick = { open(false) }) { Icon(Icons.AutoMirrored.Outlined.OpenInNew, "Öffnen") }
            Box { IconButton(onClick = { menu = true }) { Icon(Icons.Outlined.MoreVert, "Weitere Aktionen") }
                DropdownMenu(menu, { menu = false }) {
                    DropdownMenuItem({ Text("Offline behalten") }, { menu = false; space?.let { TransferScheduler.keepOffline(context, it.id, file.id, file.path) } }, leadingIcon = { Icon(Icons.Outlined.CloudDownload, null) })
                    DropdownMenuItem({ Text("Weitergeben") }, { menu = false; open(true) }, leadingIcon = { Icon(Icons.Outlined.Share, null) })
                    DropdownMenuItem({ Text("Versionen") }, { menu = false; scope.launch { runCatching { CloudApi(account).versions(file.id) }.onSuccess { versions = it }.onFailure { error = it.message } } }, leadingIcon = { Icon(Icons.Outlined.History, null) })
                    DropdownMenuItem({ Text("Freigabelink") }, { menu = false; scope.launch { runCatching {
                        val activeSpace = requireNotNull(space); val share = CloudApi(account).createReadShare(activeSpace.id, file.path)
                        val intent = Intent(Intent.ACTION_SEND).apply { type = "text/plain"; putExtra(Intent.EXTRA_TEXT, "${account.server}/${share.urlFragment}") }
                        context.startActivity(Intent.createChooser(intent, "Freigabelink senden"))
                    }.onFailure { error = it.message } } }, leadingIcon = { Icon(Icons.Outlined.Share, null) })
                    DropdownMenuItem({ Text("Löschen") }, { menu = false; delete(file) }, leadingIcon = { Icon(Icons.Outlined.Delete, null) })
                }
            }
        }
    }
    versions?.let { history -> AlertDialog(onDismissRequest = { versions = null }, confirmButton = { TextButton({ versions = null }) { Text("Schließen") } },
        title = { Text("Versionshistorie") }, text = { LazyColumn { items(history, key = FileVersion::id) { version ->
            Row(Modifier.fillMaxWidth().padding(vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) { Text(if (version.current) "Aktuelle Version" else version.createdAt); Text("${version.createdBy} · ${formatBytes(version.size)}", style = MaterialTheme.typography.bodySmall) }
                if (!version.current) TextButton({ scope.launch { runCatching { CloudApi(account).restoreVersion(file.id, version.id) }.onSuccess { versions = null; refresh() }.onFailure { error = it.message } } }) { Text("Wiederherstellen") }
            }
        } } }) }
    error?.let { message -> AlertDialog(onDismissRequest = { error = null }, confirmButton = { TextButton({ error = null }) { Text("OK") } }, title = { Text("Aktion fehlgeschlagen") }, text = { Text(message) }) }
}

@Composable
private fun TrashScreen(state: CloudUiState, selectSpace: (Space) -> Unit, restore: (CloudFile) -> Unit) {
    Column(Modifier.fillMaxSize().padding(horizontal = 16.dp)) {
      Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) { state.spaces.forEach { space ->
        if (space.id == state.selectedSpace?.id) Button({ selectSpace(space) }) { Text(space.name) }
        else OutlinedButton({ selectSpace(space) }) { Text(space.name) }
      } }
      if (state.trash.isEmpty()) EmptyMessage("Der Papierkorb ist leer.")
      else LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        items(state.trash, key = CloudFile::id) { file -> Card(Modifier.fillMaxWidth()) { Row(Modifier.fillMaxWidth().padding(14.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) { Text(file.path.substringAfterLast('/')); Text(file.path, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
            TextButton({ restore(file) }) { Text("Wiederherstellen") }
        } } }
      }
    }
}

@Composable
private fun OfflineScreen() {
    val context = androidx.compose.ui.platform.LocalContext.current
    var revision by remember { mutableIntStateOf(0) }
    val preferences = context.getSharedPreferences("offline-files", android.content.Context.MODE_PRIVATE)
    val files = remember(revision) { preferences.all.mapNotNull { (id, value) -> (value as? String)?.let(::File)?.takeIf(File::exists)?.let { id to it } } }
    if (files.isEmpty()) EmptyMessage("Noch keine Dateien offline gespeichert.")
    else LazyColumn(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        items(files, key = { it.first }) { (id, file) -> Card(Modifier.fillMaxWidth()) { Row(Modifier.fillMaxWidth().padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(file.name, Modifier.weight(1f))
            TextButton({
                val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
                val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase()) ?: "application/octet-stream"
                context.startActivity(Intent.createChooser(Intent(Intent.ACTION_VIEW).setDataAndType(uri, mime).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION), "Offline-Datei öffnen"))
            }) { Text("Öffnen") }
            TextButton({ file.delete(); preferences.edit { remove(id) }; revision += 1 }) { Text("Entfernen") }
        } } }
    }
}

@Composable
private fun SettingsScreen(spaces: List<Space>) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val settings = remember { AppSettings(context) }
    var wifiOnly by remember { mutableStateOf(settings.wifiOnly) }
    var chargingOnly by remember { mutableStateOf(settings.chargingOnly) }
    var camera by remember { mutableStateOf(settings.cameraBackupEnabled) }
    var cameraSpaceId by remember { mutableStateOf(settings.cameraSpaceId) }
    var maximumMb by remember { mutableStateOf((settings.maximumUploadBytes / 1024 / 1024).toString()) }
    var cameraFolders by remember { mutableStateOf(settings.cameraMediaFolders.joinToString(", ")) }
    var treeUri by remember { mutableStateOf(settings.treeSyncUri) }
    var treeSpaceId by remember { mutableStateOf(settings.treeSyncSpaceId) }
    var treePrefix by remember { mutableStateOf(settings.treeSyncPrefix) }
    val treePicker = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
        if (uri != null) {
            context.contentResolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
            treeUri = uri.toString(); settings.treeSyncUri = treeUri
            if (treeSpaceId == null) treeSpaceId = spaces.firstOrNull()?.id
            settings.treeSyncSpaceId = treeSpaceId; TransferScheduler.configureTreeSync(context)
        }
    }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result ->
        camera = if (Build.VERSION.SDK_INT >= 34) {
            result[Manifest.permission.READ_MEDIA_IMAGES] == true ||
                result[Manifest.permission.READ_MEDIA_VIDEO] == true ||
                result[Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED] == true
        } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            result[Manifest.permission.READ_MEDIA_IMAGES] == true ||
                result[Manifest.permission.READ_MEDIA_VIDEO] == true
        } else {
            result[Manifest.permission.READ_EXTERNAL_STORAGE] == true
        }
        settings.cameraBackupEnabled = camera
        if (cameraSpaceId == null) cameraSpaceId = spaces.firstOrNull()?.id
        settings.cameraSpaceId = cameraSpaceId
        TransferScheduler.configureCameraBackup(context)
    }
    Column(Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(18.dp)) {
        Text("Übertragungen", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
        SettingToggle("Nur über WLAN", "Mobilfunk wird nicht für Hintergrundübertragungen verwendet.", wifiOnly) {
            wifiOnly = it; settings.wifiOnly = it; TransferScheduler.configureCameraBackup(context)
        }
        SettingToggle("Nur beim Laden", "Android darf geplante Backups bis zum Laden verzögern.", chargingOnly) {
            chargingOnly = it; settings.chargingOnly = it; TransferScheduler.configureCameraBackup(context)
        }
        SettingToggle("Kamera-Backup", "Sichert DCIM/Camera. Löschen am Handy entfernt keine Cloud-Kopie.", camera) { enabled ->
            if (enabled) {
                permission.launch(
                    when {
                        Build.VERSION.SDK_INT >= 34 -> arrayOf(
                            Manifest.permission.READ_MEDIA_IMAGES,
                            Manifest.permission.READ_MEDIA_VIDEO,
                            Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED,
                        )
                        Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU -> arrayOf(
                            Manifest.permission.READ_MEDIA_IMAGES,
                            Manifest.permission.READ_MEDIA_VIDEO,
                        )
                        else -> arrayOf(Manifest.permission.READ_EXTERNAL_STORAGE)
                    },
                )
            }
            else { camera = false; settings.cameraBackupEnabled = false; TransferScheduler.configureCameraBackup(context) }
        }
        Text("Unter Android 14 kann der Zugriff auf ausgewählte Fotos und Videos begrenzt werden; dann sichert die App nur diese freigegebene Auswahl.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text("Ziel-Space", fontWeight = FontWeight.Medium)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) { spaces.forEach { space ->
            if (cameraSpaceId == space.id) Button({ cameraSpaceId = space.id; settings.cameraSpaceId = space.id; TransferScheduler.configureCameraBackup(context) }) { Text(space.name) }
            else OutlinedButton({ cameraSpaceId = space.id; settings.cameraSpaceId = space.id; TransferScheduler.configureCameraBackup(context) }) { Text(space.name) }
        } }
        OutlinedTextField(cameraFolders, { value -> cameraFolders = value; settings.cameraMediaFolders = value.split(',').map(String::trim).filter(String::isNotEmpty).map { if (it.endsWith('/')) it else "$it/" }.toSet() }, Modifier.fillMaxWidth(), label = { Text("Medienordner, mit Komma getrennt") }, supportingText = { Text("Beispiel: DCIM/Camera/, Pictures/Screenshots/") })
        OutlinedTextField(maximumMb, { value -> maximumMb = value.filter(Char::isDigit); value.toLongOrNull()?.takeIf { it > 0 }?.let { settings.maximumUploadBytes = it * 1024 * 1024 } }, Modifier.fillMaxWidth(), label = { Text("Maximale Dateigröße in MB") }, singleLine = true)
        Button({ TransferScheduler.syncNow(context) }, enabled = camera) { Text("Jetzt sichern") }
        Text("Android kann Hintergrundarbeit wegen Akku- und Systemregeln verzögern. Der manuelle Start plant sie sofort ein, garantiert aber keinen Abschluss bei geschlossener App.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text("Freigegebener Geräteordner", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
        Text(treeUri?.let { it.toUri().lastPathSegment ?: "Ordner ausgewählt" } ?: "Kein Ordner ausgewählt", color = MaterialTheme.colorScheme.onSurfaceVariant)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton({ treePicker.launch(null) }) { Text("Ordner auswählen") }
            if (treeUri != null) OutlinedButton({ settings.treeSyncUri = null; treeUri = null; TransferScheduler.configureTreeSync(context) }) { Text("Entfernen") }
        }
        Text("Ziel für Ordner-Upload", fontWeight = FontWeight.Medium)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) { spaces.forEach { space ->
            if (treeSpaceId == space.id) Button({ treeSpaceId = space.id; settings.treeSyncSpaceId = space.id; TransferScheduler.configureTreeSync(context) }) { Text(space.name) }
            else OutlinedButton({ treeSpaceId = space.id; settings.treeSyncSpaceId = space.id; TransferScheduler.configureTreeSync(context) }) { Text(space.name) }
        } }
        OutlinedTextField(treePrefix, { treePrefix = it; settings.treeSyncPrefix = it }, Modifier.fillMaxWidth(), label = { Text("Cloud-Zielordner") }, singleLine = true)
        Button({ TransferScheduler.syncTreeNow(context) }, enabled = treeUri != null && treeSpaceId != null) { Text("Ordner jetzt hochladen") }
        Text("Android gewährt nur Zugriff auf den ausdrücklich ausgewählten Ordner. Neue und geänderte Dateien werden hochgeladen; lokale Löschungen entfernen keine Cloud-Dateien. Konflikte bleiben als eigene Fassung erhalten.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun SettingToggle(title: String, description: String, checked: Boolean, changed: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) { Text(title, fontWeight = FontWeight.Medium); Text(description, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
        Switch(checked, changed)
    }
}

@Composable private fun EmptyMessage(text: String) = Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text(text, color = MaterialTheme.colorScheme.onSurfaceVariant) }

private fun queryName(context: android.content.Context, uri: Uri): String {
    context.contentResolver.query(uri, arrayOf(android.provider.OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
        if (cursor.moveToFirst()) return cursor.getString(0)
    }
    return "Datei-${System.currentTimeMillis()}"
}

private fun safeFileName(path: String): String = path.substringAfterLast('/').replace(Regex("[^A-Za-z0-9._ -]"), "_")

private fun formatBytes(bytes: Long): String = when {
    bytes >= 1_073_741_824 -> "%.1f GB".format(bytes / 1_073_741_824.0)
    bytes >= 1_048_576 -> "%.1f MB".format(bytes / 1_048_576.0)
    bytes >= 1024 -> "%.1f KB".format(bytes / 1024.0)
    else -> "$bytes B"
}

@Composable
internal fun CloudTheme(content: @Composable () -> Unit) {
    val colors = if (isSystemInDarkTheme()) androidx.compose.material3.darkColorScheme(
        primary = Color(0xFFD47A39),
        onPrimary = Color(0xFFFFFFFF),
        secondary = Color(0xFFD9B79F),
        background = Color(0xFF171513),
        onBackground = Color(0xFFEEEAE5),
        surface = Color(0xFF1C1A18),
        onSurface = Color(0xFFEEEAE5),
        surfaceVariant = Color(0xFF24211E),
        onSurfaceVariant = Color(0xFFC5BBB2),
        outline = Color(0xFF5C544D),
        outlineVariant = Color(0xFF3B3631),
        error = Color(0xFFF09289),
        errorContainer = Color(0xFF331F1E),
        onErrorContainer = Color(0xFFFFDAD6),
    ) else androidx.compose.material3.lightColorScheme(
        primary = Color(0xFF9A4F19),
        onPrimary = Color(0xFFFFFFFF),
        secondary = Color(0xFF735846),
        background = Color(0xFFFAF8F5),
        onBackground = Color(0xFF292524),
        surface = Color(0xFFFFFFFF),
        onSurface = Color(0xFF292524),
        surfaceVariant = Color(0xFFF3F0EC),
        onSurfaceVariant = Color(0xFF6B625A),
        outline = Color(0xFF9B9188),
        outlineVariant = Color(0xFFDED8D1),
        error = Color(0xFFA62B24),
        errorContainer = Color(0xFFFFF4F3),
        onErrorContainer = Color(0xFF74201B),
    )
    MaterialTheme(colorScheme = colors, content = content)
}
