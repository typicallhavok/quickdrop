package com.example.quickdrop

import java.io.File
import java.io.InputStream
import java.io.OutputStream
import java.net.Socket
import java.nio.ByteBuffer
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

class SecureChannel(val key: ByteArray) {
    var sendCtr: Long = 0
    var recvCtr: Long = 0

    // Pre-allocated read buffers
    val readLenBuf = ByteArray(4)
    val writeLenBuf = ByteArray(4)

    // Large reusable buffers to avoid per-chunk GC pressure
    // Max plaintext: header(9) + chunk(256KB) = ~262KB
    // Max encrypted: plaintext + 16 byte GCM tag
    private val MAX_PLAIN = 9 + 256 * 1024
    private val MAX_ENC = MAX_PLAIN + 16

    var writePlainBuf = ByteArray(MAX_PLAIN)
    var recvBuf = ByteArray(MAX_ENC + 1024) // receive encrypted frames

    // Reusable cipher instances
    val sendCipher: Cipher = Cipher.getInstance("AES/GCM/NoPadding")
    val recvCipher: Cipher = Cipher.getInstance("AES/GCM/NoPadding")
    val secretKey: SecretKeySpec = SecretKeySpec(key, "AES")

    // Nonce buffer reused every call
    val nonceBuf = ByteArray(12)
}

object Transfer {

    fun configureSocketForTransfer(socket: Socket) {
        try {
            socket.tcpNoDelay = true
            socket.keepAlive = true
            // Enlarge kernel socket buffers so the TCP window can grow — the
            // default ~64 KiB window otherwise caps throughput well below the
            // link rate on Wi-Fi / Wi-Fi Direct.
            socket.sendBufferSize = Protocol.SOCKET_BUFFER_SIZE
            socket.receiveBufferSize = Protocol.SOCKET_BUFFER_SIZE
        } catch (e: Exception) {
            // Ignore if platform doesn't support some options
        }
    }

    fun offerAndSendFile(
        outputStream: OutputStream,
        inputStream: InputStream,
        channel: SecureChannel,
        fileIn: InputStream,
        fileName: String,
        fileSize: Long,
        onProgress: (Long) -> Unit
    ) {
        val nameBytes = fileName.toByteArray(Charsets.UTF_8)
        val offerPayload = ByteBuffer.allocate(8 + 2 + nameBytes.size)
            .putLong(fileSize)
            .putShort(nameBytes.size.toShort())
            .put(nameBytes)
            .array()

        secureWriteFlush(outputStream, channel, Protocol.FILE_OFFER, offerPayload)

        val (msgType, _) = secureRead(inputStream, channel)
        if (msgType == Protocol.OFFER_ACCEPT) {
            secureWriteFlush(outputStream, channel, Protocol.FILE_UPLOAD, ByteArray(0))
            sendFile(outputStream, channel, fileIn, fileName, fileSize, onProgress)
        } else {
            throw Exception("Offer rejected by peer")
        }
    }

    fun sendFile(
        outputStream: OutputStream,
        channel: SecureChannel,
        fileIn: InputStream,
        fileName: String,
        fileSize: Long,
        onProgress: (Long) -> Unit
    ) {
        val nameBytes = fileName.toByteArray(Charsets.UTF_8)
        val beginPayload = ByteBuffer.allocate(8 + 2 + nameBytes.size)
            .putLong(fileSize)
            .putShort(nameBytes.size.toShort())
            .put(nameBytes)
            .array()

        secureWriteFlush(outputStream, channel, Protocol.FILE_BEGIN, beginPayload)

        val encIv = ByteArray(16)
        val sendCtr = channel.sendCtr
        encIv[8] = (sendCtr ushr 56).toByte()
        encIv[9] = (sendCtr ushr 48).toByte()
        encIv[10] = (sendCtr ushr 40).toByte()
        encIv[11] = (sendCtr ushr 32).toByte()
        encIv[12] = (sendCtr ushr 24).toByte()
        encIv[13] = (sendCtr ushr 16).toByte()
        encIv[14] = (sendCtr ushr 8).toByte()
        encIv[15] = sendCtr.toByte()

        val cipher = Cipher.getInstance("AES/CTR/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, channel.secretKey, javax.crypto.spec.IvParameterSpec(encIv))
        channel.sendCtr++

        var bytesSent = 0L

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
                    outputStream.write(buf, 0, len)
                    freeQueue.put(buf)
                }
            } catch (e: Exception) {
                threadError = e
                freeQueue.offer(ByteArray(0)) // Unblock producer
                dataQueue.clear()
            }
        }

        fileIn.use { stream ->
            while (true) {
                if (threadError != null) throw threadError!!
                val buf = freeQueue.take()
                if (threadError != null) throw threadError!!
                
                val bytesRead = stream.read(buf)
                if (bytesRead <= 0) {
                    dataQueue.put(Pair(buf, -1))
                    break
                }

                cipher.update(buf, 0, bytesRead, buf, 0)
                dataQueue.put(Pair(buf, bytesRead))
                
                bytesSent += bytesRead
                onProgress(bytesSent)
            }
            
            writerThread.join()
            if (threadError != null) throw threadError!!
            
            val finalEnc = cipher.doFinal()
            if (finalEnc != null && finalEnc.isNotEmpty()) {
                outputStream.write(finalEnc)
            }
        }

        secureWriteFlush(outputStream, channel, Protocol.FILE_END, ByteArray(0))
    }

    fun receiveFile(
        inputStream: InputStream,
        channel: SecureChannel,
        destDir: File,
        expectedSize: Long,
        expectedName: String,
        onProgress: (Long) -> Unit
    ) {
        val (msgType, payload) = secureRead(inputStream, channel)
        if (msgType != Protocol.FILE_BEGIN || payload.size < 10) {
            throw Exception("invalid data")
        }

        val buffer = ByteBuffer.wrap(payload)
        val fileSize = buffer.long
        val nameLen = buffer.short.toInt()

        if (payload.size != 10 + nameLen) {
            throw Exception("Invalid length")
        }

        val fileName = String(payload, 10, nameLen, Charsets.UTF_8)

        if (fileSize != expectedSize || fileName != expectedName) {
            throw Exception("offer mismatch")
        }

        val p = fileName.replace("\\", "/")
        if (p.contains("/") || p.contains("..")) {
            throw Exception("directory traversal in file name")
        }

        destDir.mkdirs()
        val unconfirmedFile = File(destDir, "$fileName.unconfirmed")
        val finalFile = File(destDir, fileName)

        var remaining = fileSize
        var bytesReceived = 0L

        val decIv = ByteArray(16)
        val recvCtr = channel.recvCtr
        decIv[8] = (recvCtr ushr 56).toByte()
        decIv[9] = (recvCtr ushr 48).toByte()
        decIv[10] = (recvCtr ushr 40).toByte()
        decIv[11] = (recvCtr ushr 32).toByte()
        decIv[12] = (recvCtr ushr 24).toByte()
        decIv[13] = (recvCtr ushr 16).toByte()
        decIv[14] = (recvCtr ushr 8).toByte()
        decIv[15] = recvCtr.toByte()

        val cipher = Cipher.getInstance("AES/CTR/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, channel.secretKey, javax.crypto.spec.IvParameterSpec(decIv))
        channel.recvCtr++

        val dataQueue = java.util.concurrent.ArrayBlockingQueue<Pair<ByteArray, Int>>(3)
        val freeQueue = java.util.concurrent.ArrayBlockingQueue<ByteArray>(3)

        for (i in 0 until 3) {
            freeQueue.put(ByteArray(4 * 1024 * 1024))
        }

        var threadError: Exception? = null

        val writerThread = kotlin.concurrent.thread {
            try {
                unconfirmedFile.outputStream().use { writer ->
                    while (true) {
                        val pair = dataQueue.take()
                        val buf = pair.first
                        val len = pair.second
                        if (len == -1) break
                        
                        cipher.update(buf, 0, len, buf, 0)
                        writer.write(buf, 0, len)
                        
                        freeQueue.put(buf)
                    }
                    val finalDec = cipher.doFinal()
                    if (finalDec != null && finalDec.isNotEmpty()) {
                        writer.write(finalDec)
                    }
                    writer.flush()
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
            val toRead = java.lang.Math.min(remaining, buf.size.toLong()).toInt()
            
            var totalRead = 0
            while (totalRead < toRead) {
                val count = inputStream.read(buf, totalRead, toRead - totalRead)
                if (count == -1) throw Exception("Unexpected EOF")
                totalRead += count
            }

            dataQueue.put(Pair(buf, toRead))

            remaining -= toRead
            bytesReceived += toRead
            onProgress(bytesReceived)
        }

        dataQueue.put(Pair(ByteArray(0), -1))
        writerThread.join()
        
        if (threadError != null) throw threadError!!

        val (endType, endPayload) = secureRead(inputStream, channel)
        if (endType == Protocol.FILE_END && endPayload.isEmpty() && unconfirmedFile.length() == fileSize) {
            unconfirmedFile.renameTo(finalFile)
        } else {
            throw Exception("invalid data")
        }
    }

    /**
     * Write an encrypted frame. Reuses buffers to avoid GC pressure.
     * Does NOT flush — caller must flush when appropriate (e.g. end of transfer).
     */
    fun secureWrite(
        outputStream: OutputStream,
        channel: SecureChannel,
        msgType: Byte,
        payload: ByteArray,
        offset: Int = 0,
        length: Int = payload.size
    ) {
        val header = Protocol.makeHeader(msgType, length.toLong())
        val plainLen = header.size + length

        // Grow writePlainBuf if needed (rare — only for very large control messages)
        if (channel.writePlainBuf.size < plainLen) {
            channel.writePlainBuf = ByteArray(plainLen)
        }
        val plain = channel.writePlainBuf
        System.arraycopy(header, 0, plain, 0, header.size)
        System.arraycopy(payload, offset, plain, header.size, length)

        // Build nonce in-place
        val nonce = channel.nonceBuf
        nonce[0] = 0; nonce[1] = 0; nonce[2] = 0; nonce[3] = 0
        val ctr = channel.sendCtr
        nonce[4] = (ctr ushr 56).toByte()
        nonce[5] = (ctr ushr 48).toByte()
        nonce[6] = (ctr ushr 40).toByte()
        nonce[7] = (ctr ushr 32).toByte()
        nonce[8] = (ctr ushr 24).toByte()
        nonce[9] = (ctr ushr 16).toByte()
        nonce[10] = (ctr ushr 8).toByte()
        nonce[11] = ctr.toByte()

        channel.sendCipher.init(Cipher.ENCRYPT_MODE, channel.secretKey, GCMParameterSpec(128, nonce))
        val encryptedPayload = channel.sendCipher.doFinal(plain, 0, plainLen)
        channel.sendCtr++

        val msgLenBuffer = channel.writeLenBuf
        val encLen = encryptedPayload.size
        msgLenBuffer[0] = (encLen ushr 24).toByte()
        msgLenBuffer[1] = (encLen ushr 16).toByte()
        msgLenBuffer[2] = (encLen ushr 8).toByte()
        msgLenBuffer[3] = encLen.toByte()

        outputStream.write(msgLenBuffer)
        outputStream.write(encryptedPayload)
        // NO flush here — caller controls flushing
    }

    /**
     * Flush-on-write variant for control messages (offers, accepts, etc.)
     */
    fun secureWriteFlush(
        outputStream: OutputStream,
        channel: SecureChannel,
        msgType: Byte,
        payload: ByteArray,
        offset: Int = 0,
        length: Int = payload.size
    ) {
        secureWrite(outputStream, channel, msgType, payload, offset, length)
        outputStream.flush()
    }

    fun secureRead(inputStream: InputStream, channel: SecureChannel): Pair<Byte, ByteArray> {
        val lenBuf = channel.readLenBuf
        readFully(inputStream, lenBuf, 0, 4)

        val frameLen = ((lenBuf[0].toInt() and 0xFF) shl 24) or
                ((lenBuf[1].toInt() and 0xFF) shl 16) or
                ((lenBuf[2].toInt() and 0xFF) shl 8) or
                (lenBuf[3].toInt() and 0xFF)

        if (frameLen > 100 * 1024 * 1024) throw RuntimeException("Frame too large")

        if (channel.recvBuf.size < frameLen) {
            channel.recvBuf = ByteArray(frameLen)
        }
        val encryptedBuffer = channel.recvBuf
        readFully(inputStream, encryptedBuffer, 0, frameLen)

        // Build nonce in-place
        val nonce = channel.nonceBuf
        nonce[0] = 0; nonce[1] = 0; nonce[2] = 0; nonce[3] = 0
        val ctr = channel.recvCtr
        nonce[4] = (ctr ushr 56).toByte()
        nonce[5] = (ctr ushr 48).toByte()
        nonce[6] = (ctr ushr 40).toByte()
        nonce[7] = (ctr ushr 32).toByte()
        nonce[8] = (ctr ushr 24).toByte()
        nonce[9] = (ctr ushr 16).toByte()
        nonce[10] = (ctr ushr 8).toByte()
        nonce[11] = ctr.toByte()

        channel.recvCipher.init(Cipher.DECRYPT_MODE, channel.secretKey, GCMParameterSpec(128, nonce))
        val decrypted = channel.recvCipher.doFinal(encryptedBuffer, 0, frameLen)
        channel.recvCtr++

        val headerBytes = decrypted.copyOfRange(0, Protocol.HEADER_SIZE)
        val (msgType, payloadLen) = Protocol.parseHeader(headerBytes)

        val payload = decrypted.copyOfRange(Protocol.HEADER_SIZE, Protocol.HEADER_SIZE + payloadLen.toInt())
        return Pair(msgType, payload)
    }

    /** Read exactly `len` bytes into `buf` starting at `off`. */
    private fun readFully(inputStream: InputStream, buf: ByteArray, off: Int, len: Int) {
        var totalRead = 0
        while (totalRead < len) {
            val count = inputStream.read(buf, off + totalRead, len - totalRead)
            if (count == -1) throw RuntimeException("Unexpected EOF")
            totalRead += count
        }
    }
}
