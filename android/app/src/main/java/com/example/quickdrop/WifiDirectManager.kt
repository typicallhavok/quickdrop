package com.example.quickdrop

import android.annotation.SuppressLint
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.wifi.p2p.WifiP2pConfig
import android.net.wifi.p2p.WifiP2pManager
import android.os.Looper
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.util.concurrent.atomic.AtomicBoolean

enum class WifiDirectState {
    IDLE, GROUP_CREATED, CONNECTED
}

data class WifiDirectStatus(
    val state: WifiDirectState = WifiDirectState.IDLE,
    val goIp: String? = null,
    val ssid: String = "",
    val password: String = ""
)

class WifiDirectManager(private val context: Context) {

    private var manager: WifiP2pManager? = null
    private var channel: WifiP2pManager.Channel? = null
    private var receiver: BroadcastReceiver? = null
    private var isDiscovering = false

    private val _status = MutableStateFlow(WifiDirectStatus())
    val status: StateFlow<WifiDirectStatus> = _status.asStateFlow()

    @SuppressLint("MissingPermission")
    fun start() {
        manager = context.getSystemService(Context.WIFI_P2P_SERVICE) as? WifiP2pManager
        val mgr = manager ?: return

        channel = mgr.initialize(context, Looper.getMainLooper(), null)
        val ch = channel ?: return

        receiver = object : BroadcastReceiver() {
            override fun onReceive(ctx: Context, intent: Intent) {
                when (intent.action) {
                    WifiP2pManager.WIFI_P2P_STATE_CHANGED_ACTION -> {
                        val state = intent.getIntExtra(WifiP2pManager.EXTRA_WIFI_STATE, -1)
                        if (state == WifiP2pManager.WIFI_P2P_STATE_ENABLED) {
                            startDiscovery()
                        }
                    }
                    WifiP2pManager.WIFI_P2P_DISCOVERY_CHANGED_ACTION -> {
                        val discoveryState = intent.getIntExtra(
                            WifiP2pManager.EXTRA_DISCOVERY_STATE, -1
                        )
                        if (discoveryState == WifiP2pManager.WIFI_P2P_DISCOVERY_STOPPED && isDiscovering) {
                            startDiscovery()
                        }
                    }
                    WifiP2pManager.WIFI_P2P_CONNECTION_CHANGED_ACTION -> {
                        val networkInfo = intent.getParcelableExtra<android.net.NetworkInfo>(WifiP2pManager.EXTRA_NETWORK_INFO)
                        if (networkInfo?.isConnected == true) {
                            mgr.requestConnectionInfo(ch) { info ->
                                if (info != null && info.groupFormed) {
                                    val currentIp = info.groupOwnerAddress?.hostAddress
                                    if (info.isGroupOwner) {
                                        _status.value = _status.value.copy(
                                            state = WifiDirectState.CONNECTED,
                                            goIp = currentIp
                                        )
                                    } else {
                                        _status.value = _status.value.copy(
                                            state = WifiDirectState.CONNECTED,
                                            goIp = currentIp
                                        )
                                    }
                                }
                            }
                        } else {
                            if (_status.value.state == WifiDirectState.CONNECTED) {
                                _status.value = WifiDirectStatus() // reset
                            }
                        }
                    }
                }
            }
        }

        val filter = IntentFilter().apply {
            addAction(WifiP2pManager.WIFI_P2P_STATE_CHANGED_ACTION)
            addAction(WifiP2pManager.WIFI_P2P_DISCOVERY_CHANGED_ACTION)
            addAction(WifiP2pManager.WIFI_P2P_PEERS_CHANGED_ACTION)
            addAction(WifiP2pManager.WIFI_P2P_CONNECTION_CHANGED_ACTION)
        }
        context.registerReceiver(receiver, filter)

        startDiscovery()
    }

    @SuppressLint("MissingPermission")
    private fun startDiscovery() {
        val mgr = manager ?: return
        val ch = channel ?: return
        isDiscovering = true

        mgr.discoverPeers(ch, object : WifiP2pManager.ActionListener {
            override fun onSuccess() {}
            override fun onFailure(reason: Int) {
                android.os.Handler(Looper.getMainLooper()).postDelayed({
                    if (isDiscovering) startDiscovery()
                }, 5000)
            }
        })
    }

    @SuppressLint("MissingPermission")
    fun createGroup(onComplete: (Boolean) -> Unit) {
        val mgr = manager ?: return onComplete(false)
        val ch = channel ?: return onComplete(false)
        
        val fetchGroupInfo = {
            mgr.requestGroupInfo(ch) { group ->
                if (group != null) {
                    _status.value = WifiDirectStatus(
                        state = WifiDirectState.GROUP_CREATED,
                        goIp = "192.168.49.1",
                        ssid = group.networkName ?: "",
                        password = group.passphrase ?: ""
                    )
                    onComplete(true)
                } else {
                    _status.value = WifiDirectStatus(WifiDirectState.GROUP_CREATED, "192.168.49.1")
                    onComplete(true)
                }
            }
        }

        mgr.createGroup(ch, object : WifiP2pManager.ActionListener {
            override fun onSuccess() {
                // Group creation takes a short moment to fully initialize the group info
                android.os.Handler(Looper.getMainLooper()).postDelayed({
                    fetchGroupInfo()
                }, 1000)
            }
            override fun onFailure(reason: Int) {
                // If it fails (e.g. already created), try to fetch the existing group info
                fetchGroupInfo()
            }
        })
    }
    
    @SuppressLint("MissingPermission")
    fun connectToPeer(macAddress: String, onComplete: (Boolean) -> Unit) {
        val mgr = manager ?: return onComplete(false)
        val ch = channel ?: return onComplete(false)
        
        val config = WifiP2pConfig().apply {
            deviceAddress = macAddress
        }
        
        mgr.connect(ch, config, object : WifiP2pManager.ActionListener {
            override fun onSuccess() {
                onComplete(true)
            }
            override fun onFailure(reason: Int) {
                onComplete(false)
            }
        })
    }

    @SuppressLint("MissingPermission")
    fun stop() {
        isDiscovering = false
        try {
            receiver?.let { context.unregisterReceiver(it) }
        } catch (_: Exception) {}
        
        receiver = null
        val mgr = manager
        val ch = channel
        if (mgr != null && ch != null) {
            mgr.removeGroup(ch, null)
            mgr.stopPeerDiscovery(ch, null)
        }
    }
}
