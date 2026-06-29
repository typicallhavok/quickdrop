package com.example.quickdrop

import android.content.Context
import android.os.Environment
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.io.InputStream
import java.io.OutputStream
import java.net.ServerSocket
import java.net.Socket
import java.nio.ByteBuffer
import kotlin.coroutines.resume
import kotlin.coroutines.suspendCoroutine
import kotlinx.coroutines.launch

data class ActiveSession(
    val socket: Socket,
    val secureChannel: SecureChannel,
    val inStream: InputStream,
    val outStream: OutputStream,
    val peerPublicKey: ByteArray?,
    val peerName: String,
    val peerIp: String,
    var pendingOutgoingUri: android.net.Uri? = null,
    var pendingOutgoingName: String? = null,
    var pendingOutgoingSize: Long = 0L,
    var pendingIncomingSize: Long = 0L,
    // Resume point negotiated in the accept and sent to the sender. receiveFile
    // uses exactly this (not a fresh on-disk recompute) so the body byte count
    // stays in lock-step with what the sender streams. pendingResumeEnabled
    // captures the resume setting at accept time so naming stays consistent.
    var pendingResumeOffset: Long = 0L,
    var pendingResumeEnabled: Boolean = true
) {
    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (javaClass != other?.javaClass) return false
        return socket == (other as ActiveSession).socket
    }
    override fun hashCode(): Int {
        return socket.hashCode()
    }
}

class SessionManager(
    private val context: Context,
    private val identityManager: IdentityManager,
    private val viewModel: MainViewModel,
    private val wifiDirectManager: WifiDirectManager,
    private val discoveryManager: DiscoveryManager
) {
    private val trustManager = TrustManager(context)
    private val sessionsByOfferId = java.util.concurrent.ConcurrentHashMap<String, ActiveSession>()
    private val sessionsByName = java.util.concurrent.ConcurrentHashMap<String, ActiveSession>()
    private val pendingOutgoingFiles = java.util.concurrent.ConcurrentHashMap<String, MutableList<android.net.Uri>>()

    private fun getUniqueFile(dir: File, name: String): File {
        var file = File(dir, name)
        if (!file.exists()) return file

        val dotIdx = name.lastIndexOf('.')
        val stem = if (dotIdx != -1) name.substring(0, dotIdx) else name
        val ext = if (dotIdx != -1) name.substring(dotIdx) else ""

        var i = 1
        while (true) {
            file = File(dir, "$stem ($i)$ext")
            if (!file.exists()) return file
            i++
        }
    }

    /** A name free of both an existing final file AND an existing `.unconfirmed`
     *  partial, inserting " (n)" before the extension. Used when resume is off so
     *  a fresh file is always created instead of appending to a stale partial. */
    private fun uniqueBaseName(dir: File, name: String): String {
        fun isFree(cand: String) =
            !File(dir, cand).exists() && !File(dir, "$cand.unconfirmed").exists()
        if (isFree(name)) return name
        val dotIdx = name.lastIndexOf('.')
        val stem = if (dotIdx != -1) name.substring(0, dotIdx) else name
        val ext = if (dotIdx != -1) name.substring(dotIdx) else ""
        var i = 1
        while (true) {
            val cand = "$stem ($i)$ext"
            if (isFree(cand)) return cand
            i++
        }
    }

    // Start a background server listening for connections
    suspend fun startServer(port: Int = 55432) {
        withContext(Dispatchers.IO) {
            try {
                // Set the receive buffer before bind so accepted sockets inherit
                // a large window (lets TCP throughput reach the link rate).
                val serverSocket = ServerSocket()
                try { serverSocket.receiveBufferSize = Protocol.SOCKET_BUFFER_SIZE } catch (_: Exception) {}
                serverSocket.bind(java.net.InetSocketAddress(port))
                while (true) {
                    val clientSocket = serverSocket.accept()
                    Transfer.configureSocketForTransfer(clientSocket)
                    kotlinx.coroutines.CoroutineScope(Dispatchers.IO).launch {
                        handleConnection(clientSocket, isServer = true)
                    }
                }
            } catch (_: Exception) {
            }
        }
    }

    suspend fun connectAndSend(ipAddress: String, port: Int = 55432, peerName: String, uris: List<android.net.Uri>) {
        for (uri in uris) {
            val cursor = context.contentResolver.query(uri, null, null, null, null)
            var name = "unknown"
            var size = 0L
            cursor?.use {
                if (it.moveToFirst()) {
                    val nameIdx = it.getColumnIndex(android.provider.OpenableColumns.DISPLAY_NAME)
                    val sizeIdx = it.getColumnIndex(android.provider.OpenableColumns.SIZE)
                    if (nameIdx != -1) name = it.getString(nameIdx) ?: "unknown"
                    if (sizeIdx != -1) size = it.getLong(sizeIdx)
                }
            }
            val transferId = System.currentTimeMillis().toString() + "-" + name
            withContext(Dispatchers.Main) {
                // Only skip if an in-flight transfer of this file is already running;
                // a finished/cancelled one (still briefly shown before it fades) must
                // not block resending the same file.
                val alreadyInFlight = viewModel.transfers.value.any {
                    it.fileName == name && it.peerName == peerName &&
                        (it.status == TransferStatus.ACTIVE || it.status == TransferStatus.PENDING)
                }
                if (!alreadyInFlight) {
                    // Use the real peer IP so the transfer shows under that device's
                    // card (matched by peerIp), not in a separate list.
                    viewModel.addOutgoingTransfer(transferId, name, size, peerName, ipAddress)
                }
            }
        }

        val session = connectToPeer(ipAddress, port, peerName)
        if (session != null) {
            for (uri in uris) {
                initiateFileSend(session, uri)
            }
        } else {
            if (wifiDirectManager.status.value.state == WifiDirectState.GROUP_CREATED) {
                pendingOutgoingFiles.getOrPut(peerName) { mutableListOf() }.addAll(uris)
            }
        }
    }

    suspend fun onPeerIpDiscovered(peerName: String, realIp: String) {
        val pendingUris = pendingOutgoingFiles.remove(peerName)
        if (pendingUris != null && pendingUris.isNotEmpty()) {
            connectAndSend(realIp, 55432, peerName, pendingUris)
        }
    }

    // Connect to an external peer manually (Client logic)
    suspend fun connectToPeer(ipAddress: String, port: Int = 55432, peerName: String? = null): ActiveSession? {
        return withContext(Dispatchers.IO) {
            try {
                var newSocket: Socket? = null
                if (ipAddress != "0.0.0.0") {
                    try {
                        newSocket = Socket()
                        try { newSocket.receiveBufferSize = Protocol.SOCKET_BUFFER_SIZE } catch (_: Exception) {}
                        newSocket.connect(java.net.InetSocketAddress(ipAddress, port), 2000)
                    } catch (e: Exception) {
                        newSocket = null
                    }
                }
                
                if (newSocket == null && peerName != null) {
                    // Direct connection failed, try WiFi Direct fallback
                    val peer = discoveryManager.discoveredDevices.value.find { it.name == peerName }
                    if (peer != null) {
                        val wdInfo = discoveryManager.resolveWifiDirectInfo(peer)
                        if (wdInfo != null && wdInfo.status.toInt() == 1) { // Peer is GO
                            val success = kotlin.coroutines.suspendCoroutine<Boolean> { cont ->
                                wifiDirectManager.connectToPeer(peer.macAddress) { cont.resumeWith(Result.success(it)) }
                            }
                            if (success) {
                                kotlinx.coroutines.delay(2000)
                                try {
                                    newSocket = Socket()
                                    try { newSocket.receiveBufferSize = Protocol.SOCKET_BUFFER_SIZE } catch (_: Exception) {}
                                    newSocket.connect(java.net.InetSocketAddress(wdInfo.ip, wdInfo.port), 3000)
                                } catch (_: Exception) {}
                            }
                        } else {
                            // We should become GO
                            Log.i("SessionManager", "We should become GO to share with PC. Creating Wi-Fi Direct Group...")
                            val success = kotlin.coroutines.suspendCoroutine<Boolean> { cont ->
                                wifiDirectManager.createGroup { cont.resumeWith(Result.success(it)) }
                            }
                            if (success) {
                                val status = wifiDirectManager.status.value
                                Log.i("SessionManager", "Group created successfully. SSID=${status.ssid}, IP=${status.goIp}")
                                if (status.state == WifiDirectState.GROUP_CREATED && status.ssid.isNotEmpty()) {
                                    Log.i("SessionManager", "Sending credentials to PC via BLE...")
                                    val writeSuccess = discoveryManager.sendWifiDirectCredentialsToPeer(peer, status.ssid, status.password, status.goIp ?: "192.168.49.1")
                                    if (writeSuccess) {
                                        Log.i("SessionManager", "Successfully sent credentials to PC. Waiting for PC to join hotspot and connect via TCP.")
                                        // Wait for PC to connect to us (handled by our ServerSocket)
                                        // We just keep the pending files and wait. The PC's handshake will trigger onPeerIpDiscovered.
                                        withContext(Dispatchers.Main) {
                                            viewModel.setConnectionState(true, "Waiting for PC to join hotspot...")
                                        }
                                        return@withContext null
                                    } else {
                                        Log.e("SessionManager", "Failed to send Wi-Fi Direct credentials to PC via BLE!")
                                    }
                                } else {
                                    Log.e("SessionManager", "Group created but SSID is empty or state is invalid!")
                                }
                            } else {
                                Log.e("SessionManager", "Failed to create Wi-Fi Direct Group!")
                            }
                            return@withContext null
                        }
                    }
                }
                
                if (newSocket == null) throw Exception("Connection timeout")

                Transfer.configureSocketForTransfer(newSocket)
                handleConnection(newSocket, isServer = false)
            } catch (_: Exception) {
                withContext(Dispatchers.Main) {
                    viewModel.setConnectionState(false, "Connection error")
                }
                null
            }
        }
    }

    private suspend fun handleConnection(newSocket: Socket, isServer: Boolean): ActiveSession? {
        return try {
            val inStream = newSocket.getInputStream().buffered()
            val outStream = newSocket.getOutputStream().buffered()
            
            val localPub = identityManager.getPublicKeyBytes()
            val localNameData = identityManager.localName.toByteArray()
            
            val result = if (isServer) {
                Handshake.runServerHandshake(
                    inputStream = inStream, 
                    outputStream = outStream, 
                    localPublicKey = localPub, 
                    localName = localNameData
                )
            } else {
                Handshake.runClientHandshake(
                    inputStream = inStream,
                    outputStream = outStream,
                    localPublicKey = localPub,
                    localName = localNameData,
                    signingKey = identityManager.keyPair.private
                )
            }

            val secureChannel = SecureChannel(result.sessionKey)
            val peerPublicKey = result.peerPublicKey
            val peerName = result.peerName
            val peerIp = newSocket.inetAddress.hostAddress ?: "Unknown IP"
            
            val session = ActiveSession(newSocket, secureChannel, inStream, outStream, peerPublicKey, peerName, peerIp)

            withContext(Dispatchers.Main) {
                viewModel.setConnectionState(true, peerIp)
            }
            
            if (!handleTrustFlow(session)) return null
            startListening(session)
            
            val pendingUris = pendingOutgoingFiles.remove(peerName)
            if (pendingUris != null) {
                for (uri in pendingUris) {
                    initiateFileSend(session, uri)
                }
            }
            
            session
        } catch (_: Exception) {
            newSocket.close()
            null
        }
    }
    
    private suspend fun handleTrustFlow(session: ActiveSession): Boolean {
        val pubKey = session.peerPublicKey ?: return true
        val state = trustManager.getTrustState(pubKey)
        
        if (state == TrustState.UNTRUSTED) {
            try { session.socket.close() } catch (_: Exception) {}
            return false
        }
        return true
    }

    private fun startListening(session: ActiveSession) {
        kotlinx.coroutines.CoroutineScope(Dispatchers.IO).launch {
            try {
                while (true) {
                    val (msgType, payload) = Transfer.secureRead(session.inStream, session.secureChannel)
                    
                    when (msgType) {
                        Protocol.FILE_OFFER -> handleFileOffer(session, payload)
                        Protocol.CLIPBOARD -> handleClipboard(session, payload)
                        Protocol.FILE_UPLOAD -> {
                            receiveFile(session)
                        }
                        Protocol.OFFER_ACCEPT -> {
                            var resumeOffset = 0L
                            if (payload.size == 8) {
                                resumeOffset = ByteBuffer.wrap(payload).long
                            }
                            val uri = session.pendingOutgoingUri
                            val name = session.pendingOutgoingName
                            val size = session.pendingOutgoingSize
                            if (uri != null && name != null) {
                                session.pendingOutgoingUri = null
                                withContext(Dispatchers.Main) {
                                    viewModel.markTransferActive(name)
                                }
                                kotlinx.coroutines.CoroutineScope(Dispatchers.IO).launch {
                                    performUpload(session, uri, name, size, resumeOffset)
                                }
                            }
                        }
                        Protocol.OFFER_REJECT -> {
                            val name = session.pendingOutgoingName
                            session.pendingOutgoingUri = null
                            if (name != null) {
                                withContext(Dispatchers.Main) {
                                    viewModel.markTransferRejected(name)
                                }
                            }
                        }
                        else -> { }
                    }
                }
            } catch (_: Exception) {
                cleanupSession(session)
            }
        }
    }

    private suspend fun handleFileOffer(session: ActiveSession, payload: ByteArray) {
        if (payload.size < 10) return
        val buffer = ByteBuffer.wrap(payload)
        val fileSize = buffer.long
        val nameLen = buffer.short.toInt()
        
        if (payload.size < 10 + nameLen) return
        val fileName = String(payload, 10, nameLen, Charsets.UTF_8)
        
        val offerId = System.currentTimeMillis().toString() + "-" + fileName
        session.pendingOutgoingName = fileName
        session.pendingIncomingSize = fileSize
        sessionsByOfferId[offerId] = session
        sessionsByName[fileName] = session
        
        val isTrusted = session.peerPublicKey?.let { trustManager.getTrustState(it) == TrustState.TRUSTED } == true

        withContext(Dispatchers.Main) {
            if (isTrusted) {
                viewModel.autoAcceptOffer(offerId, fileName, fileSize, session.peerName, session.peerIp)
            } else {
                viewModel.addOffer(IncomingOffer(id = offerId, fileName = fileName, fileSize = fileSize, peerName = session.peerName, peerIp = session.peerIp, peerPublicKey = session.peerPublicKey))
            }
        }
        
        if (isTrusted) {
            sendOfferAccept(offerId)
        }
    }

    /** A peer pushed clipboard text — copy it into the system clipboard and toast. */
    private suspend fun handleClipboard(session: ActiveSession, payload: ByteArray) {
        val text = String(payload, Charsets.UTF_8)
        withContext(Dispatchers.Main) {
            try {
                val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
                clipboard.setPrimaryClip(android.content.ClipData.newPlainText("Quickdrop", text))
            } catch (_: Exception) {}
            android.widget.Toast.makeText(
                context,
                "Clipboard copied from ${session.peerName}",
                android.widget.Toast.LENGTH_SHORT
            ).show()
        }
    }

    /** Connect to a peer and push the given text to its clipboard. */
    suspend fun sendClipboard(ipAddress: String, port: Int = 55432, peerName: String, text: String) {
        val session = connectToPeer(ipAddress, port, peerName)
        if (session != null) {
            withContext(Dispatchers.IO) {
                try {
                    Transfer.secureWriteFlush(
                        session.outStream,
                        session.secureChannel,
                        Protocol.CLIPBOARD,
                        text.toByteArray(Charsets.UTF_8)
                    )
                } catch (e: Exception) {
                    Log.e("SessionManager", "Failed to send clipboard: ${e.message}")
                }
            }
        }
    }

    /**
     * Decide where an accepted transfer should resume and bring the on-disk
     * `.unconfirmed` file into agreement. Single source of truth for the resume
     * offset — `receiveFile` recomputes the offset from the file length, so the
     * file state set here is what it sees. Returns the offset (0 == fresh).
     */
    private fun prepareResumeOffset(tempFile: File, expectedSize: Long): Long {
        if (!tempFile.exists()) {
            android.util.Log.d("Quickdrop", "[resume] accept: no partial -> fresh (0)")
            return 0L
        }
        val existing = tempFile.length()
        // Unusable (corrupt / complete-but-unconfirmed) or too small to bother.
        if (existing == 0L || existing >= expectedSize || existing < Protocol.RESUME_MIN_BYTES) {
            android.util.Log.d("Quickdrop", "[resume] accept: partial=$existing expected=$expectedSize unusable/too-small -> discard, fresh (0)")
            tempFile.delete()
            return 0L
        }
        // Resume, but rewind a little and re-receive the tail so a torn write at
        // the end of the partial is overwritten rather than trusted as-is.
        val offset = (existing - Protocol.RESUME_REWIND_BYTES).coerceAtLeast(0L)
        return try {
            java.io.RandomAccessFile(tempFile, "rw").use { it.setLength(offset) }
            android.util.Log.d("Quickdrop", "[resume] accept: partial=$existing expected=$expectedSize -> RESUME from $offset")
            offset
        } catch (_: Exception) {
            tempFile.delete()
            0L
        }
    }

    suspend fun sendOfferAccept(offerId: String) {
        withContext(Dispatchers.IO) {
            val session = sessionsByOfferId[offerId] ?: return@withContext
            sessionsByOfferId.remove(offerId)
            var payload = ByteArray(0)
            var offset = 0L
            val fileName = session.pendingOutgoingName
            val resumeEnabled = AppSettings.resumeTransfers(context)
            // When resume is disabled we never continue a partial: tell the sender
            // to start at 0 and leave any stale `.unconfirmed` untouched (a fresh,
            // uniquely-named file is written on receive instead).
            if (fileName != null && resumeEnabled) {
                val publicDownloads = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
                val downloadDir = File(publicDownloads, "Quickdrop")
                val tempFile = File(downloadDir, "$fileName.unconfirmed")
                offset = prepareResumeOffset(tempFile, session.pendingIncomingSize)
                if (offset > 0L) {
                    payload = java.nio.ByteBuffer.allocate(8).putLong(offset).array()
                }
            }
            // Record the negotiated offset/resume flag so receiveFile reads from
            // exactly the same point we told the sender to start streaming from.
            session.pendingResumeOffset = offset
            session.pendingResumeEnabled = resumeEnabled
            Transfer.secureWriteFlush(session.outStream, session.secureChannel, Protocol.OFFER_ACCEPT, payload)
        }
    }

    suspend fun sendOfferReject(offerId: String) {
        withContext(Dispatchers.IO) {
            val session = sessionsByOfferId[offerId] ?: return@withContext
            sessionsByOfferId.remove(offerId)
            Transfer.secureWriteFlush(session.outStream, session.secureChannel, Protocol.OFFER_REJECT, ByteArray(0))
        }
    }

    suspend fun initiateFileSend(session: ActiveSession, uri: android.net.Uri) {
        val cursor = context.contentResolver.query(uri, null, null, null, null)
        var name = "unknown"
        var size = 0L
        cursor?.use {
            if (it.moveToFirst()) {
                val nameIdx = it.getColumnIndex(android.provider.OpenableColumns.DISPLAY_NAME)
                val sizeIdx = it.getColumnIndex(android.provider.OpenableColumns.SIZE)
                if (nameIdx != -1) name = it.getString(nameIdx) ?: "unknown"
                if (sizeIdx != -1) size = it.getLong(sizeIdx)
            }
        }
        
        session.pendingOutgoingUri = uri
        session.pendingOutgoingName = name
        session.pendingOutgoingSize = size

        val transferId = System.currentTimeMillis().toString() + "-" + name
        withContext(Dispatchers.Main) {
            if (viewModel.transfers.value.none { it.fileName == name && it.peerName == session.peerName }) {
                viewModel.addOutgoingTransfer(transferId, name, size, session.peerName, session.peerIp)
            }
        }
        sessionsByName[name] = session

        val nameBytes = name.toByteArray(Charsets.UTF_8)
        val buffer = ByteBuffer.allocate(8 + 2 + nameBytes.size)
            .putLong(size)
            .putShort(nameBytes.size.toShort())
            .put(nameBytes)

        withContext(Dispatchers.IO) {
            Transfer.secureWriteFlush(session.outStream, session.secureChannel, Protocol.FILE_OFFER, buffer.array())
        }
    }

    private suspend fun performUpload(session: ActiveSession, uri: android.net.Uri, name: String, size: Long, resumeOffset: Long = 0L) {
        try {
            // Send FILE_UPLOAD control message (flush)
            Transfer.secureWriteFlush(session.outStream, session.secureChannel, Protocol.FILE_UPLOAD, ByteArray(0))
            
            val nameBytes = name.toByteArray(Charsets.UTF_8)
            val beginPayload = ByteBuffer.allocate(8 + 2 + nameBytes.size)
                .putLong(size)
                .putShort(nameBytes.size.toShort())
                .put(nameBytes)
                .array()
            
            // FILE_BEGIN — flush so receiver gets it immediately
            Transfer.secureWriteFlush(session.outStream, session.secureChannel, Protocol.FILE_BEGIN, beginPayload)

            val inS = context.contentResolver.openInputStream(uri) ?: throw Exception("Cannot open file")
            val dataQueue = java.util.concurrent.ArrayBlockingQueue<Pair<ByteArray, Int>>(3)
            val freeQueue = java.util.concurrent.ArrayBlockingQueue<ByteArray>(3)

            for (i in 0 until 3) {
                freeQueue.put(ByteArray(4 * 1024 * 1024))
            }

            var threadError: Exception? = null

            val writerThread = kotlin.concurrent.thread {
                try {
                    while (true) {
                        val pair = dataQueue.take()
                        val buf = pair.first
                        val len = pair.second
                        if (len == -1) break
                        
                        session.outStream.write(buf, 0, len)
                        freeQueue.put(buf)
                    }
                } catch (e: Exception) {
                    threadError = e
                    freeQueue.offer(ByteArray(0))
                    dataQueue.clear()
                }
            }

            var bytesDone = resumeOffset
            var lastProgressUpdate = System.currentTimeMillis()
            var lastCancelCheck = System.currentTimeMillis()
            
            val encIv = ByteArray(16)
            val sendCtr = session.secureChannel.sendCtr
            encIv[8] = (sendCtr ushr 56).toByte()
            encIv[9] = (sendCtr ushr 48).toByte()
            encIv[10] = (sendCtr ushr 40).toByte()
            encIv[11] = (sendCtr ushr 32).toByte()
            encIv[12] = (sendCtr ushr 24).toByte()
            encIv[13] = (sendCtr ushr 16).toByte()
            encIv[14] = (sendCtr ushr 8).toByte()
            encIv[15] = sendCtr.toByte()

            val cipher = javax.crypto.Cipher.getInstance("AES/CTR/NoPadding")
            cipher.init(javax.crypto.Cipher.ENCRYPT_MODE, session.secureChannel.secretKey, javax.crypto.spec.IvParameterSpec(encIv))
            session.secureChannel.sendCtr++

            inS.use { stream ->
                if (resumeOffset > 0) {
                    // skip() may return fewer bytes than requested — loop until done.
                    var toSkip = resumeOffset
                    while (toSkip > 0) {
                        val skipped = stream.skip(toSkip)
                        if (skipped <= 0) {
                            // Fall back to reading-and-discarding if skip stalls.
                            val tmp = ByteArray(minOf(toSkip, 1024L * 1024).toInt())
                            val r = stream.read(tmp)
                            if (r <= 0) throw Exception("Could not seek to resume offset")
                            toSkip -= r
                        } else {
                            toSkip -= skipped
                        }
                    }
                }
                while(true) {
                    if (threadError != null) throw threadError!!
                    val buf = freeQueue.take()
                    if (threadError != null) throw threadError!!

                    // Throttle cancel check to every 500ms
                    val now = System.currentTimeMillis()
                    if (now - lastCancelCheck > 500) {
                        lastCancelCheck = now
                        val currentTransfer = viewModel.transfers.value.find { it.fileName == name }
                        if (currentTransfer?.status == TransferStatus.CANCELLED) {
                            throw Exception("Transfer cancelled by user")
                        }
                    }

                    val read = stream.read(buf)
                    if (read <= 0) {
                        dataQueue.put(Pair(buf, -1))
                        break
                    }

                    cipher.update(buf, 0, read, buf, 0)
                    dataQueue.put(Pair(buf, read))

                    bytesDone += read
                    
                    // Throttle UI updates to roughly every 200ms
                    val nowProgress = System.currentTimeMillis()
                    if (nowProgress - lastProgressUpdate > 200 || bytesDone == size) {
                        lastProgressUpdate = nowProgress
                        // Post update without suspending — avoid context switch overhead
                        val done = bytesDone
                        kotlinx.coroutines.CoroutineScope(Dispatchers.Main).launch {
                            viewModel.updateTransferProgress(name, done)
                        }
                    }
                }
                
                writerThread.join()
                if (threadError != null) throw threadError!!
                
                val finalEnc = cipher.doFinal()
                if (finalEnc != null && finalEnc.isNotEmpty()) {
                    session.outStream.write(finalEnc)
                }
            }
            
            // Flush remaining buffered data before FILE_END
            session.outStream.flush()
            Transfer.secureWriteFlush(session.outStream, session.secureChannel, Protocol.FILE_END, ByteArray(0))
            withContext(Dispatchers.Main) {
                viewModel.markTransferDone(name)
            }
        } catch(e: Exception) {
            withContext(Dispatchers.Main) {
                viewModel.markTransferError(name)
            }
            cleanupSession(session)
        }
    }

    private suspend fun receiveFile(session: ActiveSession) {
        var currentFileName: String? = null
        // Signals the writer thread to stop when the receive is interrupted, so it
        // doesn't block forever on dataQueue.take() holding the partial file open.
        val aborted = java.util.concurrent.atomic.AtomicBoolean(false)

        try {
            val (msgType, payload) = Transfer.secureRead(session.inStream, session.secureChannel)
            if (msgType != Protocol.FILE_BEGIN || payload.size < 10) {
                throw Exception("Expected FILE_BEGIN")
            }

            val buffer = ByteBuffer.wrap(payload)
            val fileSize = buffer.long
            val nameLen = buffer.short.toInt()
            val fileName = String(payload, 10, nameLen, Charsets.UTF_8)
            currentFileName = fileName

            val publicDownloads = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
            val downloadDir = File(publicDownloads, "Quickdrop")
            downloadDir.mkdirs()
            
            // With resume enabled (default) reuse the `<name>.unconfirmed` partial.
            // With resume disabled, pick a fresh unique base so a stale partial is
            // never appended to — the result lands as a new "(n)" file.
            val resume = session.pendingResumeEnabled
            val baseName = if (resume) fileName else uniqueBaseName(downloadDir, fileName)
            val tempFile = File(downloadDir, "$baseName.unconfirmed")
            val finalFile = if (resume) getUniqueFile(downloadDir, fileName) else File(downloadDir, baseName)

            // The resume point was negotiated in sendOfferAccept and sent to the
            // sender, who streams exactly `fileSize - offset` bytes. Treat it as
            // authoritative and force the partial to exactly that length, rather
            // than recomputing from the on-disk length: the two can disagree (e.g.
            // if the accept-time truncation didn't persist), and any mismatch makes
            // us read the wrong number of body bytes — corrupting the FILE_END
            // frame, throwing, and dropping the socket (which the sender sees as a
            // failure). Forcing the length keeps both sides in lock-step.
            val existingLength = if (resume) session.pendingResumeOffset else 0L
            java.io.RandomAccessFile(tempFile, "rw").use { it.setLength(existingLength) }
            val bytesToReceive = if (fileSize > existingLength) fileSize - existingLength else 0L
            var remaining = bytesToReceive
            val outStream = java.io.FileOutputStream(tempFile, true)

            var lastProgressUpdate = System.currentTimeMillis()
            var lastCancelCheck = System.currentTimeMillis()

            val decIv = ByteArray(16)
            val recvCtr = session.secureChannel.recvCtr
            decIv[8] = (recvCtr ushr 56).toByte()
            decIv[9] = (recvCtr ushr 48).toByte()
            decIv[10] = (recvCtr ushr 40).toByte()
            decIv[11] = (recvCtr ushr 32).toByte()
            decIv[12] = (recvCtr ushr 24).toByte()
            decIv[13] = (recvCtr ushr 16).toByte()
            decIv[14] = (recvCtr ushr 8).toByte()
            decIv[15] = recvCtr.toByte()

            val cipher = javax.crypto.Cipher.getInstance("AES/CTR/NoPadding")
            cipher.init(javax.crypto.Cipher.DECRYPT_MODE, session.secureChannel.secretKey, javax.crypto.spec.IvParameterSpec(decIv))
            session.secureChannel.recvCtr++

            val dataQueue = java.util.concurrent.ArrayBlockingQueue<Pair<ByteArray, Int>>(3)
            val freeQueue = java.util.concurrent.ArrayBlockingQueue<ByteArray>(3)

            for (i in 0 until 3) {
                freeQueue.put(ByteArray(4 * 1024 * 1024))
            }

            var threadError: Exception? = null

            val writerThread = kotlin.concurrent.thread {
                try {
                    outStream.use { fileOut ->
                        while (true) {
                            // Poll (don't block forever) so an interrupted receive
                            // can tell this thread to stop and release the file.
                            val pair = dataQueue.poll(200, java.util.concurrent.TimeUnit.MILLISECONDS)
                                ?: if (aborted.get()) break else continue
                            val buf = pair.first
                            val len = pair.second
                            if (len == -1) break

                            cipher.update(buf, 0, len, buf, 0)
                            fileOut.write(buf, 0, len)
                            freeQueue.put(buf)
                        }
                        val decFinal = cipher.doFinal()
                        if (decFinal != null && decFinal.isNotEmpty()) {
                            fileOut.write(decFinal)
                        }
                    }
                } catch (e: Exception) {
                    threadError = e
                    freeQueue.offer(ByteArray(0))
                    dataQueue.clear()
                }
            }

            while (remaining > 0) {
                if (threadError != null) throw threadError!!
                val buf = freeQueue.take()
                if (threadError != null) throw threadError!!

                // Throttle cancel check
                val now = System.currentTimeMillis()
                if (now - lastCancelCheck > 500) {
                    lastCancelCheck = now
                    val currentTransfer = viewModel.transfers.value.find { it.fileName == fileName }
                    if (currentTransfer?.status == TransferStatus.CANCELLED) {
                        throw Exception("Transfer cancelled by user")
                    }
                }

                val toRead = java.lang.Math.min(remaining, buf.size.toLong()).toInt()
                var totalRead = 0
                while (totalRead < toRead) {
                    val count = session.inStream.read(buf, totalRead, toRead - totalRead)
                    if (count == -1) throw Exception("Unexpected EOF")
                    totalRead += count
                }

                dataQueue.put(Pair(buf, toRead))
                
                remaining -= toRead
                    
                // Throttle progress updates to every 200ms
                val nowProgress = System.currentTimeMillis()
                val done = existingLength + (bytesToReceive - remaining)
                if (nowProgress - lastProgressUpdate > 200 || remaining == 0L) {
                    lastProgressUpdate = nowProgress
                    kotlinx.coroutines.CoroutineScope(Dispatchers.Main).launch {
                        viewModel.updateTransferProgress(fileName, done)
                    }
                }
            }
            
            dataQueue.put(Pair(ByteArray(0), -1))
            writerThread.join()
            if (threadError != null) throw threadError!!

            val (endType, endPayload) = Transfer.secureRead(session.inStream, session.secureChannel)
            android.util.Log.d("Quickdrop", "[resume] receive end: name=$fileName existingLength=$existingLength expected=$fileSize endType=0x${endType.toString(16)} endPayloadLen=${endPayload.size} tempLen=${tempFile.length()}")
            if (endType == Protocol.FILE_END && endPayload.isEmpty()) {
                if (tempFile.length() == fileSize) {
                    val finalOk = tempFile.renameTo(finalFile)
                    withContext(Dispatchers.Main) {
                        if (finalOk) {
                            viewModel.markTransferDone(fileName)
                        } else {
                            viewModel.markTransferError(fileName)
                        }
                    }
                } else {
                    throw Exception("File size mismatch")
                }
            } else {
                throw Exception("Expected FILE_END")
            }

        } catch (e: Exception) {
            android.util.Log.w("Quickdrop", "[resume] receive failed for ${currentFileName}: ${e.message}")
            // Release the writer thread so it stops blocking and closes the partial
            // file handle; otherwise the next send of this file collides with it.
            aborted.set(true)
            currentFileName?.let { name ->
                withContext(Dispatchers.Main) {
                    viewModel.markTransferError(name)
                }
            }
            cleanupSession(session)
        }
    }

    private fun cleanupSession(session: ActiveSession) {
        try {
            session.socket.close()
        } catch (_: Exception) {
        }
    }
}
