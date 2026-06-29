package com.example.quickdrop

import android.content.Context
import android.net.wifi.WifiManager
import kotlinx.coroutines.*
import java.net.*
import java.util.concurrent.atomic.AtomicBoolean

class UdpDiscoveryManager(
    private val context: Context,
    private val onDeviceDiscovered: (ip: String, port: Int, name: String) -> Unit
) {

    private val DISCOVERY_PORT = 55433
    private val PREFIX = "QUICKDROP_DISCOVER:"

    private var socket: DatagramSocket? = null
    private val isRunning = AtomicBoolean(false)
    private var multicastLock: WifiManager.MulticastLock? = null

    private fun getWifiIp(): String? {
        val wm = context.applicationContext
            .getSystemService(Context.WIFI_SERVICE) as WifiManager

        val ip = wm.connectionInfo.ipAddress
        if (ip == 0) return null

        return String.format(
            "%d.%d.%d.%d",
            ip and 0xff,
            ip shr 8 and 0xff,
            ip shr 16 and 0xff,
            ip shr 24 and 0xff
        )
    }

    private fun getWifiBroadcast(): InetAddress? {
        val wm = context.applicationContext
            .getSystemService(Context.WIFI_SERVICE) as WifiManager

        val dhcp = wm.dhcpInfo ?: return null
        val broadcast = (dhcp.ipAddress and dhcp.netmask) or dhcp.netmask.inv()

        val quads = ByteArray(4)
        for (k in 0..3) {
            quads[k] = (broadcast shr (k * 8)).toByte()
        }

        return InetAddress.getByAddress(quads)
    }

    suspend fun start(deviceName: String, tcpPort: Int = 55432) = withContext(Dispatchers.IO) {
        if (isRunning.getAndSet(true)) return@withContext

        val wm = context.applicationContext
            .getSystemService(Context.WIFI_SERVICE) as WifiManager

        multicastLock = wm.createMulticastLock("quickdrop_lock").apply {
            setReferenceCounted(true)
            acquire()
        }

        val wifiIp = getWifiIp()
        val broadcastAddr = getWifiBroadcast()

        if (wifiIp == null || broadcastAddr == null) {
            return@withContext
        }

        try {
            socket = DatagramSocket(null).apply {
                reuseAddress = true
                broadcast = true
                bind(InetSocketAddress(wifiIp, DISCOVERY_PORT))
            }

            val message = "$PREFIX$deviceName:$tcpPort".toByteArray()

            // Sender
            val senderJob = launch {
                while (isRunning.get()) {
                    try {
                        socket?.send(
                            DatagramPacket(
                                message,
                                message.size,
                                broadcastAddr,
                                DISCOVERY_PORT
                            )
                        )
                    } catch (_: Exception) {
                    }
                    delay(2000)
                }
            }

            // Receiver
            val buf = ByteArray(1024)

            while (isRunning.get()) {
                try {
                    val packet = DatagramPacket(buf, buf.size)
                    socket?.receive(packet)

                    val msg = String(packet.data, 0, packet.length)
                    val senderIp = packet.address.hostAddress ?: continue

                    if (senderIp == wifiIp) continue
                    


                    if (!msg.startsWith(PREFIX)) continue

                    val payload = msg.removePrefix(PREFIX)
                    val colonPos = payload.lastIndexOf(':')
                    val name: String
                    val port: Int
                    if (colonPos != -1) {
                        name = payload.substring(0, colonPos)
                        port = payload.substring(colonPos + 1).toIntOrNull() ?: 55432
                    } else {
                        name = payload
                        port = 55432
                    }

                    onDeviceDiscovered(senderIp, port, name)

                    val reply = "$PREFIX$deviceName:$tcpPort".toByteArray()
                    socket?.send(
                        DatagramPacket(
                            reply,
                            reply.size,
                            packet.address,
                            DISCOVERY_PORT
                        )
                    )

                } catch (_: Exception) {
                }
            }

            senderJob.cancel()

        } catch (_: Exception) {
        } finally {
            stop()
        }
    }

    fun stop() {
        isRunning.set(false)
        socket?.close()
        socket = null
        multicastLock?.release()
        multicastLock = null
    }
}