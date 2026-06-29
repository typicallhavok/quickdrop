package com.example.quickdrop

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Share
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.lifecycleScope
import com.example.quickdrop.ui.theme.QuickdropTheme
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {

    private val viewModel: MainViewModel by viewModels()
    private lateinit var identityManager: IdentityManager
    private lateinit var sessionManager: SessionManager
    private lateinit var udpDiscoveryManager: UdpDiscoveryManager
    private lateinit var discoveryManager: DiscoveryManager
    private lateinit var wifiDirectManager: WifiDirectManager

    private val resolvedMacs = mutableSetOf<String>()
    private val resolvingMacs = mutableSetOf<String>() // Add this line to track active attempts

    private val filePickerLauncher = registerForActivityResult(ActivityResultContracts.GetMultipleContents()) { uris ->
        for (uri in uris) {
            addFileFromUri(uri)
        }
    }

    private fun addFileFromUri(uri: Uri) {
        var name = "unknown"
        var size = 0L
        contentResolver.query(uri, null, null, null, null)?.use { cursor ->
            if (cursor.moveToFirst()) {
                val nameIdx = cursor.getColumnIndex(android.provider.OpenableColumns.DISPLAY_NAME)
                val sizeIdx = cursor.getColumnIndex(android.provider.OpenableColumns.SIZE)
                if (nameIdx != -1) name = cursor.getString(nameIdx) ?: "unknown"
                if (sizeIdx != -1) size = cursor.getLong(sizeIdx)
            }
        }
        viewModel.addSelectedFile(SelectedFile(uri, name, size))
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleIntent(intent)
    }

    private fun handleIntent(intent: Intent) {
        val action = intent.action
        val type = intent.type

        if (Intent.ACTION_SEND == action && type != null) {
            val uri = intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM)
            if (uri != null) {
                addFileFromUri(uri)
            }
        } else if (Intent.ACTION_SEND_MULTIPLE == action && type != null) {
            val uris = intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM)
            if (uris != null) {
                for (uri in uris) {
                    addFileFromUri(uri)
                }
            }
        }
    }

    private var collectionStarted = false

    private fun startDiscovery() {
        discoveryManager.startAdvertising()
        discoveryManager.startScanning()

        if (collectionStarted) return
        collectionStarted = true

        lifecycleScope.launch {
            discoveryManager.discoveredDevices.collect { devices ->
                for (dev in devices) {
                    // Skip if already found or if an active resolution attempt is running
                    if (dev.macAddress in resolvedMacs || dev.macAddress in resolvingMacs) continue

                    resolvingMacs.add(dev.macAddress)

                    // Launch concurrently so it doesn't freeze the list loop
                    lifecycleScope.launch {
                        val ip = discoveryManager.resolveDeviceIp(dev)
                        resolvingMacs.remove(dev.macAddress) // Resolution attempt finished

                        if (ip != null) {
                            resolvedMacs.add(dev.macAddress) // Permanently mark as resolved only on success
                            viewModel.addDiscoveredPeer(DiscoveredPeer(dev.name, ip, 55432))
                        }
                    }
                }
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        identityManager = IdentityManager(this)
        wifiDirectManager = WifiDirectManager(this)
        discoveryManager = DiscoveryManager(this, wifiDirectManager)
        sessionManager = SessionManager(this, identityManager, viewModel, wifiDirectManager, discoveryManager)
        udpDiscoveryManager = UdpDiscoveryManager(this) { ip, port, name ->
            viewModel.addDiscoveredPeer(DiscoveredPeer(name, ip, port))
            lifecycleScope.launch {
                sessionManager.onPeerIpDiscovered(name, ip)
            }
        }

        handleIntent(intent)
        wifiDirectManager.start()

        lifecycleScope.launch {
            sessionManager.startServer()
        }

        lifecycleScope.launch {
            udpDiscoveryManager.start(identityManager.localName)
        }

        startDiscovery()

        setContent {
            QuickdropTheme {
                Surface(
                    modifier = Modifier.fillMaxSize(),
                    color = MaterialTheme.colorScheme.background
                ) {
                    QuickDropApp(
                        viewModel = viewModel,
                        onPickFile = { filePickerLauncher.launch("*/*") },
                        onAcceptOffer = { offerId ->
                            viewModel.acceptOffer(offerId)
                            lifecycleScope.launch { sessionManager.sendOfferAccept(offerId) }
                        },
                        onTrustAndAcceptOffer = { offerId, pubKey ->
                            if (pubKey != null) {
                                val offer = viewModel.incomingOffers.value.find { it.id == offerId }
                                val peerName = offer?.peerName ?: "Unknown Device"
                                TrustManager(this@MainActivity).setTrustState(pubKey, true, peerName)
                            }
                            viewModel.acceptOffer(offerId)
                            lifecycleScope.launch { sessionManager.sendOfferAccept(offerId) }
                        },
                        onRejectOffer = { offerId ->
                            viewModel.rejectOffer(offerId)
                            lifecycleScope.launch { sessionManager.sendOfferReject(offerId) }
                        },
                        onConnectToPeer = { peer ->
                            val files = viewModel.selectedFiles.value
                            if (files.isNotEmpty()) {
                                val uris = files.map { it.uri }
                                lifecycleScope.launch {
                                    sessionManager.connectAndSend(peer.ip, peer.port, peer.name, uris)
                                }
                                for (file in files) {
                                    viewModel.clearSelectedFile(file.name)
                                }
                            }
                        },
                        onSendClipboard = { peer ->
                            val clipboard = getSystemService(android.content.Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
                            val text = clipboard.primaryClip?.getItemAt(0)?.coerceToText(this@MainActivity)?.toString()
                            if (!text.isNullOrEmpty()) {
                                lifecycleScope.launch {
                                    sessionManager.sendClipboard(peer.ip, peer.port, peer.name, text)
                                }
                                android.widget.Toast.makeText(this@MainActivity, "Clipboard sent", android.widget.Toast.LENGTH_SHORT).show()
                            } else {
                                android.widget.Toast.makeText(this@MainActivity, "Clipboard is empty", android.widget.Toast.LENGTH_SHORT).show()
                            }
                        },
                        onPermissionsGranted = {
                            startDiscovery()
                        }
                    )
                }
            }
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        wifiDirectManager.stop()
        udpDiscoveryManager.stop()
        discoveryManager.stopAdvertising()
        discoveryManager.stopScanning()
    }
}

@OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
@Composable
fun QuickDropApp(
    viewModel: MainViewModel,
    onPickFile: () -> Unit,
    onAcceptOffer: (String) -> Unit,
    onTrustAndAcceptOffer: (String, ByteArray?) -> Unit,
    onRejectOffer: (String) -> Unit,
    onConnectToPeer: (DiscoveredPeer) -> Unit,
    onSendClipboard: (DiscoveredPeer) -> Unit,
    onPermissionsGranted: () -> Unit
) {
    var showSettings by remember { mutableStateOf(false) }

    if (showSettings) {
        // Intercept the system back gesture/button so it returns to the main
        // screen instead of falling through to the Activity (which closes the app).
        BackHandler { showSettings = false }
        SettingsScreen(onClose = { showSettings = false })
        return
    }

    val permissionLauncher = androidx.activity.compose.rememberLauncherForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.RequestMultiplePermissions()
    ) { results ->
        onPermissionsGranted()
    }

    LaunchedEffect(Unit) {
        val permissions = mutableListOf(
            android.Manifest.permission.ACCESS_FINE_LOCATION,
            android.Manifest.permission.ACCESS_COARSE_LOCATION,
            android.Manifest.permission.READ_EXTERNAL_STORAGE,
            android.Manifest.permission.WRITE_EXTERNAL_STORAGE
        )
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU) {
            permissions.add(android.Manifest.permission.NEARBY_WIFI_DEVICES)
            permissions.add(android.Manifest.permission.POST_NOTIFICATIONS)
        }
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.S) {
            permissions.add(android.Manifest.permission.BLUETOOTH_CONNECT)
            permissions.add(android.Manifest.permission.BLUETOOTH_SCAN)
            permissions.add(android.Manifest.permission.BLUETOOTH_ADVERTISE)
        }
        permissionLauncher.launch(permissions.toTypedArray())
    }

    val isConnected by viewModel.isConnected.collectAsState()
    val connectedPeer by viewModel.connectedPeer.collectAsState()
    val discoveredPeers by viewModel.discoveredPeers.collectAsState()
    val incomingOffers by viewModel.incomingOffers.collectAsState()
    val transfers by viewModel.transfers.collectAsState()
    val selectedFiles by viewModel.selectedFiles.collectAsState()

    val darkBg = MaterialTheme.colorScheme.background
    val cyan = MaterialTheme.colorScheme.primaryContainer
    val lightText = MaterialTheme.colorScheme.onSurface
    val iconGray = MaterialTheme.colorScheme.onSurfaceVariant

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(darkBg)
    ) {
        TopAppBar(
            modifier = Modifier
                .fillMaxWidth()
                .background(darkBg),
            title = {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Icon(
                        imageVector = Icons.Default.Share,
                        contentDescription = "Quickdrop",
                        tint = cyan,
                        modifier = Modifier.size(28.dp)
                    )
                    Spacer(modifier = Modifier.width(8.dp))
                    Text(
                        "Quickdrop",
                        color = cyan,
                        fontSize = 20.sp,
                        fontWeight = FontWeight.Bold
                    )
                    Spacer(modifier = Modifier.weight(1f))
                    IconButton(onClick = { showSettings = true }) {
                        Icon(
                            imageVector = Icons.Default.Settings,
                            contentDescription = "Settings",
                            tint = iconGray,
                            modifier = Modifier.size(24.dp)
                        )
                    }
                }
            },
            colors = TopAppBarDefaults.topAppBarColors(
                containerColor = darkBg,
                titleContentColor = lightText
            )
        )

        LazyColumn(
            modifier = Modifier
                .fillMaxSize()
                .background(darkBg)
                .padding(16.dp),
            verticalArrangement = Arrangement.Top
        ) {
            if (incomingOffers.isNotEmpty()) {
                item {
                    Text(
                        "INCOMING SHARING REQUESTS",
                        color = cyan,
                        fontSize = 16.sp,
                        fontWeight = FontWeight.Bold,
                        modifier = Modifier.padding(vertical = 12.dp)
                    )
                }
                items(incomingOffers) { offer ->
                    Card(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(bottom = 8.dp),
                        colors = CardDefaults.cardColors(
                            containerColor = MaterialTheme.colorScheme.surfaceContainerHigh
                        )
                    ) {
                        Column(modifier = Modifier.padding(16.dp)) {
                            Text(
                                offer.fileName,
                                color = cyan,
                                fontWeight = FontWeight.SemiBold,
                                fontSize = 16.sp,
                                maxLines = 1,
                                modifier = Modifier.fillMaxWidth()
                            )
                            Text(
                                "${offer.fileSize} bytes from ${offer.peerName}",
                                color = iconGray,
                                fontSize = 12.sp,
                                modifier = Modifier.padding(top = 4.dp, bottom = 12.dp)
                            )
                            Row(modifier = Modifier.fillMaxWidth()) {
                                Button(
                                    onClick = { onAcceptOffer(offer.id) },
                                    modifier = Modifier.weight(1f).padding(end = 4.dp),
                                    colors = ButtonDefaults.buttonColors(containerColor = cyan)
                                ) {
                                    Text("Accept", color = darkBg, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                                }
                                Button(
                                    onClick = { onTrustAndAcceptOffer(offer.id, offer.peerPublicKey) },
                                    modifier = Modifier.weight(1f).padding(horizontal = 4.dp),
                                    colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.surfaceContainerHighest)
                                ) {
                                    Text("Always Trust", color = lightText, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                                }
                                OutlinedButton(
                                    onClick = { onRejectOffer(offer.id) },
                                    modifier = Modifier.weight(1f).padding(start = 4.dp).border(1.dp, MaterialTheme.colorScheme.error.copy(alpha=0.5f), shape = MaterialTheme.shapes.small),
                                    colors = ButtonDefaults.outlinedButtonColors()
                                ) {
                                    Text("Reject", color = MaterialTheme.colorScheme.error, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                                }
                            }
                        }
                    }
                }
            }

            if (selectedFiles.isNotEmpty()) {
                item {
                    Text(
                        "SELECTED FILES",
                        color = cyan,
                        fontSize = 16.sp,
                        fontWeight = FontWeight.Bold,
                        modifier = Modifier.padding(top = 8.dp, bottom = 12.dp)
                    )
                }
                items(selectedFiles) { file ->
                    Card(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(bottom = 8.dp),
                        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow)
                    ) {
                        Row(modifier = Modifier.padding(12.dp).fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                            Column(modifier = Modifier.weight(1f)) {
                                Text(file.name, color = lightText, fontWeight = FontWeight.Medium, fontSize = 14.sp)
                                Text("${file.size} bytes", color = iconGray, fontSize = 12.sp)
                            }
                            IconButton(onClick = { viewModel.removeSelectedFile(file.uri) }) {
                                Icon(imageVector = Icons.Default.Close, contentDescription = "Remove", tint = MaterialTheme.colorScheme.error)
                            }
                        }
                    }
                }
            }

            item {
                Spacer(modifier = Modifier.height(8.dp))
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .border(1.dp, cyan, shape = MaterialTheme.shapes.medium)
                        .background(MaterialTheme.colorScheme.surfaceContainerLow, shape = MaterialTheme.shapes.medium)
                        .clickable { onPickFile() }
                        .padding(24.dp),
                    contentAlignment = Alignment.Center
                ) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Icon(imageVector = Icons.Default.Info, contentDescription = "Upload", tint = cyan)
                        Spacer(modifier = Modifier.height(8.dp))
                        Text(
                            "Select Files to Share",
                            color = cyan,
                            fontSize = 14.sp,
                            fontWeight = FontWeight.SemiBold
                        )
                    }
                }
            }

            item {
                Text(
                    "NEARBY DEVICES",
                    color = cyan,
                    fontSize = 16.sp,
                    fontWeight = FontWeight.Bold,
                    modifier = Modifier.padding(top = 32.dp, bottom = 12.dp)
                )
            }

            items(discoveredPeers) { peer ->
                val isThisConnected = isConnected && peer.ip == connectedPeer
                val hasFilesToDrop = selectedFiles.isNotEmpty()
                val peerTransfers = transfers.filter { it.peerIp == peer.ip }

                Card(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(bottom = 12.dp)
                        .clickable(enabled = hasFilesToDrop) {
                            onConnectToPeer(peer)
                        },
                    colors = CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.surfaceContainerLow
                    ),
                    elevation = CardDefaults.cardElevation(
                        defaultElevation = if (hasFilesToDrop) 4.dp else 0.dp
                    )
                ) {
                    Column(modifier = Modifier.fillMaxWidth()) {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(16.dp),
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Box(
                                modifier = Modifier
                                    .size(12.dp)
                                    .background(
                                        color = if (isThisConnected) Color(0xFF00FF00) else Color.Gray,
                                        shape = androidx.compose.foundation.shape.CircleShape
                                    )
                            )
                            Spacer(modifier = Modifier.width(16.dp))
                            Column(modifier = Modifier.weight(1f)) {
                                Text(
                                    peer.name,
                                    color = lightText,
                                    fontWeight = FontWeight.Bold,
                                    fontSize = 16.sp
                                )
                                Text(
                                    "Nearby",
                                    color = cyan,
                                    fontSize = 12.sp,
                                    fontWeight = FontWeight.Medium
                                )
                            }
                            TextButton(onClick = { onSendClipboard(peer) }) {
                                Text("Clipboard", color = cyan, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                            }
                            if (hasFilesToDrop) {
                                Icon(imageVector = Icons.Default.Share, contentDescription = "Send", tint = cyan)
                            }
                        }

                        if (peerTransfers.isNotEmpty()) {
                            Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
                                for (transfer in peerTransfers) {
                                    val pct = if (transfer.fileSize > 0) ((transfer.bytesDone.toDouble() / transfer.fileSize.toDouble()) * 100).toInt() else 0
                                    val inProgress = transfer.status == TransferStatus.ACTIVE || transfer.status == TransferStatus.PENDING
                                    Row(modifier = Modifier.fillMaxWidth().padding(bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                                        Text(transfer.fileName, color = lightText, fontSize = 12.sp, modifier = Modifier.weight(1f), maxLines = 1)
                                        when (transfer.status) {
                                            TransferStatus.ACTIVE, TransferStatus.PENDING -> {
                                                Text("${pct}%", color = cyan, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                                                IconButton(
                                                    onClick = { viewModel.cancelTransfer(transfer.id) },
                                                    modifier = Modifier.size(24.dp).padding(start = 8.dp)
                                                ) {
                                                    Icon(imageVector = Icons.Default.Close, contentDescription = "Cancel", tint = MaterialTheme.colorScheme.error, modifier = Modifier.size(16.dp))
                                                }
                                            }
                                            TransferStatus.DONE -> Text(
                                                if (transfer.direction == TransferDirection.RECEIVE) "Saved" else "Sent",
                                                color = cyan, fontSize = 12.sp, fontWeight = FontWeight.Bold
                                            )
                                            TransferStatus.ERROR -> Text("Failed", color = MaterialTheme.colorScheme.error, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                                            TransferStatus.CANCELLED -> Text("Cancelled", color = iconGray, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                                            TransferStatus.REJECTED -> Text("Rejected", color = MaterialTheme.colorScheme.error, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                                        }
                                    }
                                    if (inProgress) {
                                        LinearProgressIndicator(
                                            progress = { pct / 100f },
                                            modifier = Modifier.fillMaxWidth().padding(bottom = 12.dp),
                                            color = cyan,
                                            trackColor = MaterialTheme.colorScheme.surfaceContainerHighest
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
            }

            item {
                Spacer(modifier = Modifier.height(32.dp))
            }
        }
    }
}