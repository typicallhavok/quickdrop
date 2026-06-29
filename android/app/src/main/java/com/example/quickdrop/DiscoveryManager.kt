package com.example.quickdrop

import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattServer
import android.bluetooth.BluetoothGattServerCallback
import android.bluetooth.BluetoothGattService
import android.bluetooth.BluetoothManager
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertisingSet
import android.bluetooth.le.AdvertisingSetCallback
import android.bluetooth.le.AdvertisingSetParameters
import android.bluetooth.le.BluetoothLeAdvertiser
import android.bluetooth.le.BluetoothLeScanner
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.net.wifi.WifiManager
import android.os.ParcelUuid
import kotlinx.coroutines.Dispatchers
import android.util.Log
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import java.util.UUID
import kotlin.coroutines.resume

data class DiscoveredDevice(
    val name: String,
    val macAddress: String,
    val bluetoothDevice: BluetoothDevice
)

class DiscoveryManager(
    private val context: Context,
    private val wifiDirectManager: WifiDirectManager
) {
    private val bluetoothManager: BluetoothManager = context.getSystemService(Context.BLUETOOTH_SERVICE) as BluetoothManager
    private val bluetoothAdapter: BluetoothAdapter? get() = bluetoothManager.adapter
    private val leScanner: BluetoothLeScanner? get() = bluetoothAdapter?.bluetoothLeScanner
    private val leAdvertiser: BluetoothLeAdvertiser? get() = bluetoothAdapter?.bluetoothLeAdvertiser

    private val TAG = "DiscoveryManager"

    private val _discoveredDevices = MutableStateFlow<List<DiscoveredDevice>>(emptyList())
    val discoveredDevices: StateFlow<List<DiscoveredDevice>> = _discoveredDevices.asStateFlow()

    private val SERVICE_UUID_STR = "00001d09-0000-1000-8000-00805f9b34fc"
    private val WIFI_INFO_UUID_STR = "00001d0a-0000-1000-8000-00805f9b34fc"
    private val WIFIDIRECT_INFO_UUID_STR = "00001d0b-0000-1000-8000-00805f9b34fc"
    private val WIFIDIRECT_CONTROL_UUID_STR = "00001d0c-0000-1000-8000-00805f9b34fc"

    private val SERVICE_UUID = ParcelUuid(UUID.fromString(SERVICE_UUID_STR))
    private val WIFI_INFO_UUID = UUID.fromString(WIFI_INFO_UUID_STR)
    private val WIFIDIRECT_INFO_UUID = UUID.fromString(WIFIDIRECT_INFO_UUID_STR)
    private val WIFIDIRECT_CONTROL_UUID = UUID.fromString(WIFIDIRECT_CONTROL_UUID_STR)

    private var gattServer: BluetoothGattServer? = null
    private var isScanning = false
    private var isAdvertising = false

    private val scanCallback = object : ScanCallback() {
        @SuppressLint("MissingPermission")
        override fun onScanResult(callbackType: Int, result: ScanResult) {
            super.onScanResult(callbackType, result)

            val scanRecord = result.scanRecord ?: return
            val serviceUuids = scanRecord.serviceUuids
            val isOurService = serviceUuids?.contains(SERVICE_UUID) == true

            if (!isOurService) {
                return
            }

            val deviceName = result.device.name ?: scanRecord.deviceName ?: "Unknown Device"
            // Log removed

            val currentList = _discoveredDevices.value.toMutableList()
            val existingIndex = currentList.indexOfFirst { it.macAddress == result.device.address }

            val newDevice = DiscoveredDevice(
                name = deviceName,
                macAddress = result.device.address,
                bluetoothDevice = result.device
            )

            if (existingIndex == -1) {
                currentList.add(newDevice)
            } else {
                currentList[existingIndex] = newDevice
            }
            _discoveredDevices.value = currentList
        }
    }

    private var currentAdvertisingSet: AdvertisingSet? = null

    private val advertisingSetCallback = object : AdvertisingSetCallback() {
        override fun onAdvertisingSetStarted(advertisingSet: AdvertisingSet?, txPower: Int, status: Int) {
            currentAdvertisingSet = advertisingSet
        }

        override fun onAdvertisingSetStopped(advertisingSet: AdvertisingSet?) {
            currentAdvertisingSet = null
        }
    }

    private fun getLocalIpBytes(): ByteArray {
        val wifiManager = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        val ipAddress = wifiManager.connectionInfo.ipAddress
        return byteArrayOf(
            (ipAddress and 0xFF).toByte(),
            ((ipAddress shr 8) and 0xFF).toByte(),
            ((ipAddress shr 16) and 0xFF).toByte(),
            ((ipAddress shr 24) and 0xFF).toByte()
        )
    }

    private fun getWifiDirectInfoBytes(): ByteArray {
        val wdStatus = wifiDirectManager.status.value
        val statusByte: Byte = when (wdStatus.state) {
            WifiDirectState.IDLE -> 0
            WifiDirectState.GROUP_CREATED -> 1
            WifiDirectState.CONNECTED -> 2
        }
        val ipBytes = if (wdStatus.goIp != null) {
            val parts = wdStatus.goIp.split(".")
            if (parts.size == 4) {
                byteArrayOf(
                    parts[0].toInt().toByte(),
                    parts[1].toInt().toByte(),
                    parts[2].toInt().toByte(),
                    parts[3].toInt().toByte()
                )
            } else byteArrayOf(0, 0, 0, 0)
        } else {
            byteArrayOf(0, 0, 0, 0)
        }
        val portBytes = byteArrayOf((55432 shr 8).toByte(), (55432 and 0xFF).toByte())
        
        val ssidBytes = wdStatus.ssid.toByteArray(Charsets.UTF_8)
        val passBytes = wdStatus.password.toByteArray(Charsets.UTF_8)
        val ssidLen = ssidBytes.size.toByte()
        val passLen = passBytes.size.toByte()
        
        return byteArrayOf(statusByte) + ipBytes + portBytes + byteArrayOf(ssidLen) + ssidBytes + byteArrayOf(passLen) + passBytes
    }

    @SuppressLint("MissingPermission")
    fun startAdvertising() {
        if (isAdvertising) return
        val advertiser = leAdvertiser ?: return

        setupGattServer()

        val parameters = AdvertisingSetParameters.Builder()
            .setLegacyMode(true)
            .setConnectable(true)
            .setScannable(true)
            .setInterval(AdvertisingSetParameters.INTERVAL_LOW)
            .setTxPowerLevel(AdvertisingSetParameters.TX_POWER_HIGH)
            .build()

        val data = AdvertiseData.Builder()
            .setIncludeDeviceName(true)
            .addServiceUuid(SERVICE_UUID)
            .build()

        try {
            advertiser.startAdvertisingSet(parameters, data, null, null, null, advertisingSetCallback)
            isAdvertising = true
        } catch (e: SecurityException) {
            Log.e(TAG, "SecurityException starting advertising: ${e.message}")
        } catch (e: Exception) {
            Log.e(TAG, "Exception starting advertising: ${e.message}")
        }
    }

    @SuppressLint("MissingPermission")
    private fun setupGattServer() {
        if (gattServer != null) return

        val gattServerCallback = object : BluetoothGattServerCallback() {
            override fun onCharacteristicReadRequest(
                device: BluetoothDevice?,
                requestId: Int,
                offset: Int,
                characteristic: BluetoothGattCharacteristic?
            ) {
                val fullPayload = when (characteristic?.uuid) {
                    WIFI_INFO_UUID -> getLocalIpBytes()
                    WIFIDIRECT_INFO_UUID -> getWifiDirectInfoBytes()
                    else -> null
                }
                
                if (fullPayload != null) {
                    if (offset > fullPayload.size) {
                        gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_INVALID_OFFSET, offset, null)
                    } else {
                        val valueToSend = fullPayload.copyOfRange(offset, fullPayload.size)
                        gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, valueToSend)
                    }
                } else {
                    gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_REQUEST_NOT_SUPPORTED, offset, null)
                }
            }
            override fun onCharacteristicWriteRequest(
                device: BluetoothDevice?,
                requestId: Int,
                characteristic: BluetoothGattCharacteristic?,
                preparedWrite: Boolean,
                responseNeeded: Boolean,
                offset: Int,
                value: ByteArray?
            ) {
                if (characteristic?.uuid == WIFIDIRECT_CONTROL_UUID) {
                    if (value != null && value.isNotEmpty() && value[0] == 1.toByte()) {
                        wifiDirectManager.createGroup { _ -> }
                    }
                    if (responseNeeded) {
                        gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, value)
                    }
                } else {
                    if (responseNeeded) {
                        gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_REQUEST_NOT_SUPPORTED, offset, null)
                    }
                }
            }
        }

        try {
            gattServer = bluetoothManager.openGattServer(context, gattServerCallback)
            if (gattServer == null) return

            val service = BluetoothGattService(UUID.fromString(SERVICE_UUID_STR), BluetoothGattService.SERVICE_TYPE_PRIMARY)
            val charWifi = BluetoothGattCharacteristic(
                WIFI_INFO_UUID,
                BluetoothGattCharacteristic.PROPERTY_READ,
                BluetoothGattCharacteristic.PERMISSION_READ
            )
            val charWd = BluetoothGattCharacteristic(
                WIFIDIRECT_INFO_UUID,
                BluetoothGattCharacteristic.PROPERTY_READ,
                BluetoothGattCharacteristic.PERMISSION_READ
            )
            val charCtrl = BluetoothGattCharacteristic(
                WIFIDIRECT_CONTROL_UUID,
                BluetoothGattCharacteristic.PROPERTY_WRITE,
                BluetoothGattCharacteristic.PERMISSION_WRITE
            )
            service.addCharacteristic(charWifi)
            service.addCharacteristic(charWd)
            service.addCharacteristic(charCtrl)
            gattServer?.addService(service)
        } catch (e: SecurityException) {
            Log.e(TAG, "SecurityException opening GATT Server: ${e.message}")
        } catch (e: Exception) {
            Log.e(TAG, "Exception setupGattServer: ${e.message}")
        }
    }

    @SuppressLint("MissingPermission")
    fun stopAdvertising() {
        try {
            gattServer?.close()
            gattServer = null
            if (currentAdvertisingSet != null) {
                leAdvertiser?.stopAdvertisingSet(advertisingSetCallback)
                currentAdvertisingSet = null
            }
        } catch (e: Exception) {
            Log.e(TAG, "Exception stopping advertising: ${e.message}")
        } finally {
            isAdvertising = false
        }
    }

    @SuppressLint("MissingPermission")
    fun startScanning() {
        if (isScanning) return
        val scanner = leScanner
        if (scanner == null) {
            Log.e(TAG, "BLE Scanner not available")
            return
        }

        // Open filter payload structure to capture custom split service data fragments across Windows blocks
        val filter = ScanFilter.Builder().build()

        val settings = ScanSettings.Builder()
            .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
            .build()

        try {
            Log.d(TAG, "Starting BLE scan...")
            _discoveredDevices.value = emptyList()
            scanner.startScan(listOf(filter), settings, scanCallback)
            isScanning = true
        } catch (e: SecurityException) {
            Log.e(TAG, "SecurityException starting scan: ${e.message}")
        } catch (e: Exception) {
            Log.e(TAG, "Exception starting scan: ${e.message}")
        }
    }

    @SuppressLint("MissingPermission")
    fun stopScanning() {
        if (!isScanning) return
        try {
            leScanner?.stopScan(scanCallback)
        } catch (e: Exception) {
            Log.e(TAG, "Exception stopping scan: ${e.message}")
        } finally {
            isScanning = false
        }
    }

    @SuppressLint("MissingPermission")
    suspend fun resolveDeviceIp(device: DiscoveredDevice): String? = withContext(Dispatchers.IO) {
        withTimeoutOrNull(10_000L) {
            suspendCancellableCoroutine { cont ->
                var gatt: BluetoothGatt? = null
                Log.d(TAG, "[BLE_GATT] Attempting to connect to ${device.name}...")

                val callback = object : BluetoothGattCallback() {
                    @SuppressLint("MissingPermission")
                    override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
                        Log.d(TAG, "[BLE_GATT] Connection state changed: status=$status, newState=$newState")
                        if (newState == android.bluetooth.BluetoothProfile.STATE_CONNECTED) {
                            Log.d(TAG, "[BLE_GATT] Connected! Discovering services...")
                            gatt.discoverServices()
                        } else if (newState == android.bluetooth.BluetoothProfile.STATE_DISCONNECTED) {
                            Log.e(TAG, "[BLE_GATT] Disconnected. (Status: $status)")
                            if (cont.isActive) cont.resume(null)
                            gatt.close()
                        }
                    }

                    @SuppressLint("MissingPermission")
                    override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
                        Log.d(TAG, "[BLE_GATT] Services discovered status: $status")
                        if (status == BluetoothGatt.GATT_SUCCESS) {
                            val service = gatt.getService(UUID.fromString(SERVICE_UUID_STR))
                            if (service == null) {
                                Log.e(TAG, "[BLE_GATT] Target Service NOT found!")
                                if (cont.isActive) cont.resume(null)
                                gatt.disconnect()
                                return
                            }

                            val characteristic = service.getCharacteristic(WIFI_INFO_UUID)
                            if (characteristic != null) {
                                Log.d(TAG, "[BLE_GATT] Characteristic found! Reading...")
                                gatt.readCharacteristic(characteristic)
                            } else {
                                Log.e(TAG, "[BLE_GATT] Characteristic NOT found!")
                                if (cont.isActive) cont.resume(null)
                                gatt.disconnect()
                            }
                        } else {
                            if (cont.isActive) cont.resume(null)
                            gatt.disconnect()
                        }
                    }

                    @Suppress("DEPRECATION")
                    @SuppressLint("MissingPermission")
                    override fun onCharacteristicRead(
                        gatt: BluetoothGatt,
                        characteristic: BluetoothGattCharacteristic,
                        status: Int
                    ) {
                        Log.d(TAG, "[BLE_GATT] Read characteristic status: $status")
                        if (status == BluetoothGatt.GATT_SUCCESS && characteristic.uuid == WIFI_INFO_UUID) {
                            val data = characteristic.value
                            if (data != null && data.size >= 4) {
                                val ip = "${data[0].toUByte()}.${data[1].toUByte()}.${data[2].toUByte()}.${data[3].toUByte()}"
                                Log.d(TAG, "[BLE_GATT] Successfully read IP: $ip")
                                if (cont.isActive) cont.resume(ip)
                            } else {
                                Log.e(TAG, "[BLE_GATT] Data was null or too short")
                                if (cont.isActive) cont.resume(null)
                            }
                        } else {
                            if (cont.isActive) cont.resume(null)
                        }
                        try {
                            gatt.disconnect()
                        } catch (_: Exception) {}
                    }
                }

                try {
                    // CRITICAL FIX: Force TRANSPORT_LE (Low Energy) so Windows doesn't reject it
                    if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.M) {
                        gatt = device.bluetoothDevice.connectGatt(context, false, callback, BluetoothDevice.TRANSPORT_LE)
                    } else {
                        gatt = device.bluetoothDevice.connectGatt(context, false, callback)
                    }
                } catch (e: Exception) {
                    Log.e(TAG, "[BLE_GATT] Exception calling connectGatt: ${e.message}")
                    if (cont.isActive) cont.resume(null)
                }

                cont.invokeOnCancellation {
                    try {
                        gatt?.disconnect()
                        gatt?.close()
                    } catch (_: Exception) {}
                }
            }
        } ?: "0.0.0.0"
    }
    data class WifiDirectInfo(val status: Byte, val ip: String, val port: Int)
    @SuppressLint("MissingPermission")
    suspend fun resolveWifiDirectInfo(device: DiscoveredDevice): WifiDirectInfo? = withContext(Dispatchers.IO) {
        withTimeoutOrNull(10_000L) {
            suspendCancellableCoroutine { cont ->
                var gatt: BluetoothGatt? = null
                val callback = object : BluetoothGattCallback() {
                    @SuppressLint("MissingPermission")
                    override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
                        if (newState == android.bluetooth.BluetoothProfile.STATE_CONNECTED) {
                            gatt.discoverServices()
                        } else if (newState == android.bluetooth.BluetoothProfile.STATE_DISCONNECTED) {
                            if (cont.isActive) cont.resume(null)
                            gatt.close()
                        }
                    }

                    @SuppressLint("MissingPermission")
                    override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
                        if (status == BluetoothGatt.GATT_SUCCESS) {
                            val service = gatt.getService(UUID.fromString(SERVICE_UUID_STR))
                            val characteristic = service?.getCharacteristic(WIFIDIRECT_INFO_UUID)
                            if (characteristic != null) {
                                gatt.readCharacteristic(characteristic)
                            } else {
                                if (cont.isActive) cont.resume(null)
                                gatt.disconnect()
                            }
                        } else {
                            if (cont.isActive) cont.resume(null)
                            gatt.disconnect()
                        }
                    }

                    @Suppress("DEPRECATION")
                    @SuppressLint("MissingPermission")
                    override fun onCharacteristicRead(
                        gatt: BluetoothGatt,
                        characteristic: BluetoothGattCharacteristic,
                        status: Int
                    ) {
                        if (status == BluetoothGatt.GATT_SUCCESS && characteristic.uuid == WIFIDIRECT_INFO_UUID) {
                            val data = characteristic.value
                            if (data != null && data.size >= 7) {
                                val st = data[0]
                                val ip = "${data[1].toUByte()}.${data[2].toUByte()}.${data[3].toUByte()}.${data[4].toUByte()}"
                                val port = ((data[5].toInt() and 0xFF) shl 8) or (data[6].toInt() and 0xFF)
                                if (cont.isActive) cont.resume(WifiDirectInfo(st, ip, port))
                            } else {
                                if (cont.isActive) cont.resume(null)
                            }
                        } else {
                            if (cont.isActive) cont.resume(null)
                        }
                        try {
                            gatt.disconnect()
                        } catch (_: Exception) {}
                    }
                }
                try {
                    gatt = device.bluetoothDevice.connectGatt(context, false, callback)
                } catch (e: SecurityException) {
                    if (cont.isActive) cont.resume(null)
                } catch (e: Exception) {
                    if (cont.isActive) cont.resume(null)
                }
                cont.invokeOnCancellation {
                    try {
                        gatt?.disconnect()
                        gatt?.close()
                    } catch (_: Exception) {}
                }
            }
        }
    }

    @SuppressLint("MissingPermission")
    suspend fun sendWifiDirectCredentialsToPeer(device: DiscoveredDevice, ssid: String, pass: String, goIp: String): Boolean = withContext(Dispatchers.IO) {
        withTimeoutOrNull(10_000L) {
            suspendCancellableCoroutine { cont ->
                var gatt: BluetoothGatt? = null
                val callback = object : BluetoothGattCallback() {
                    @SuppressLint("MissingPermission")
                    override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
                        Log.d(TAG, "[BLE_CRED_WRITE] onConnectionStateChange: status=$status, newState=$newState")
                        if (newState == android.bluetooth.BluetoothProfile.STATE_CONNECTED) {
                            Log.i(TAG, "[BLE_CRED_WRITE] Connected to PC GATT server, discovering services...")
                            gatt.discoverServices()
                        } else if (newState == android.bluetooth.BluetoothProfile.STATE_DISCONNECTED) {
                            Log.w(TAG, "[BLE_CRED_WRITE] Disconnected from PC GATT server")
                            if (cont.isActive) cont.resume(false)
                            gatt.close()
                        }
                    }

                    @SuppressLint("MissingPermission")
                    override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
                        Log.d(TAG, "[BLE_CRED_WRITE] onServicesDiscovered: status=$status")
                        if (status == BluetoothGatt.GATT_SUCCESS) {
                            val service = gatt.getService(UUID.fromString(SERVICE_UUID_STR))
                            val characteristic = service?.getCharacteristic(WIFIDIRECT_CONTROL_UUID)
                            if (characteristic != null) {
                                Log.i(TAG, "[BLE_CRED_WRITE] Found CONTROL characteristic, writing credentials...")
                                // Build payload: [4 bytes GO_IP, 1 byte ssid_len, ssid, 1 byte pass_len, pass]
                                val ipParts = goIp.split(".")
                                val ipBytes = if (ipParts.size == 4) {
                                    byteArrayOf(ipParts[0].toInt().toByte(), ipParts[1].toInt().toByte(), ipParts[2].toInt().toByte(), ipParts[3].toInt().toByte())
                                } else byteArrayOf(0, 0, 0, 0)
                                
                                val ssidBytes = ssid.toByteArray(Charsets.UTF_8)
                                val passBytes = pass.toByteArray(Charsets.UTF_8)
                                
                                val payload = ipBytes + byteArrayOf(ssidBytes.size.toByte()) + ssidBytes + byteArrayOf(passBytes.size.toByte()) + passBytes
                                Log.d(TAG, "[BLE_CRED_WRITE] Payload size=${payload.size}, SSID=$ssid")
                                
                                characteristic.value = payload
                                gatt.writeCharacteristic(characteristic)
                            } else {
                                Log.e(TAG, "[BLE_CRED_WRITE] CONTROL characteristic NOT found! Service=${service != null}")
                                if (cont.isActive) cont.resume(false)
                                gatt.disconnect()
                            }
                        } else {
                            Log.e(TAG, "[BLE_CRED_WRITE] Service discovery failed with status=$status")
                            if (cont.isActive) cont.resume(false)
                            gatt.disconnect()
                        }
                    }

                    @SuppressLint("MissingPermission")
                    override fun onCharacteristicWrite(
                        gatt: BluetoothGatt,
                        characteristic: BluetoothGattCharacteristic,
                        status: Int
                    ) {
                        Log.d(TAG, "[BLE_CRED_WRITE] onCharacteristicWrite: status=$status, uuid=${characteristic.uuid}")
                        if (status == BluetoothGatt.GATT_SUCCESS && characteristic.uuid == WIFIDIRECT_CONTROL_UUID) {
                            Log.i(TAG, "[BLE_CRED_WRITE] Successfully wrote credentials to PC!")
                            if (cont.isActive) cont.resume(true)
                        } else {
                            Log.e(TAG, "[BLE_CRED_WRITE] Write failed! status=$status")
                            if (cont.isActive) cont.resume(false)
                        }
                        try {
                            gatt.disconnect()
                        } catch (_: Exception) {}
                    }
                }
                try {
                    if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.M) {
                        gatt = device.bluetoothDevice.connectGatt(context, false, callback, BluetoothDevice.TRANSPORT_LE)
                    } else {
                        gatt = device.bluetoothDevice.connectGatt(context, false, callback)
                    }
                } catch (e: Exception) {
                    Log.e(TAG, "[BLE_GATT] Exception calling connectGatt for credential write: ${e.message}")
                    if (cont.isActive) cont.resume(false)
                }
                cont.invokeOnCancellation {
                    try {
                        gatt?.disconnect()
                        gatt?.close()
                    } catch (_: Exception) {}
                }
            }
        } ?: false
    }
}