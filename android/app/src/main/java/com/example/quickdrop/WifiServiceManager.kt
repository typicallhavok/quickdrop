package com.example.quickdrop

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.p2p.WifiP2pConfig
import android.net.wifi.p2p.WifiP2pManager
import android.os.Looper
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.Socket

class WifiServiceManager(private val context: Context) {
    private val nsdManager = context.getSystemService(Context.NSD_SERVICE) as NsdManager
    private val wifiP2pManager = context.getSystemService(Context.WIFI_P2P_SERVICE) as WifiP2pManager
    private val channel = wifiP2pManager.initialize(context, Looper.getMainLooper(), null)
    
    private val SERVICE_TYPE = "_quickdrop._tcp."
    private val SERVICE_NAME = "QuickDrop_Android"
    private var registrationListener: NsdManager.RegistrationListener? = null

    fun registerService(port: Int) {
        val serviceInfo = NsdServiceInfo().apply {
            serviceName = SERVICE_NAME
            serviceType = SERVICE_TYPE
            setPort(port)
        }

        registrationListener = object : NsdManager.RegistrationListener {
            override fun onServiceRegistered(NsdServiceInfo: NsdServiceInfo) {}
            override fun onRegistrationFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {}
            override fun onServiceUnregistered(arg0: NsdServiceInfo) {}
            override fun onUnregistrationFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {}
        }
        
        nsdManager.registerService(serviceInfo, NsdManager.PROTOCOL_DNS_SD, registrationListener)
    }

    suspend fun attemptConnection(ipAddress: String, port: Int): Socket? = withContext(Dispatchers.IO) {
        try {
            val socket = Socket()
            socket.connect(InetSocketAddress(ipAddress, port), 3000)
            return@withContext socket
        } catch (_: Exception) {
            val inetAddr = InetAddress.getByName(ipAddress)
            val bytes = inetAddr.address
            val firstByte = bytes[0].toInt() and 0xFF

            if (firstByte == 2) {
                connectViaWifiP2p(ipAddress)
            }
            
            return@withContext null
        }
    }

    private fun connectViaWifiP2p(deviceAddress: String) {
        val config = WifiP2pConfig().apply {
            this.deviceAddress = deviceAddress
        }
        try {
            wifiP2pManager.connect(channel, config, object : WifiP2pManager.ActionListener {
                override fun onSuccess() {}
                override fun onFailure(reason: Int) {}
            })
        } catch (_: SecurityException) {
        }
    }

    fun stopService() {
        registrationListener?.let { nsdManager.unregisterService(it) }
    }
}