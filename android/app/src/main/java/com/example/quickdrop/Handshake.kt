package com.example.quickdrop

import java.io.InputStream
import java.io.OutputStream
import java.nio.ByteBuffer
import java.security.KeyFactory
import java.security.MessageDigest
import java.security.PrivateKey
import java.security.SecureRandom
import java.security.Signature
import java.security.spec.X509EncodedKeySpec

data class HandshakeResult(
    val sessionKey: ByteArray,
    val peerPublicKey: ByteArray,
    val peerName: String
)

object Handshake {

    const val NONCE_SIZE = 32

    fun generateNonce(): ByteArray {
        val nonce = ByteArray(NONCE_SIZE)
        SecureRandom().nextBytes(nonce)
        return nonce
    }

    private fun addEd25519Prefix(rawKey: ByteArray): ByteArray {
        val prefix = byteArrayOf(
            0x30.toByte(), 0x2A.toByte(), 0x30.toByte(), 0x05.toByte(),
            0x06.toByte(), 0x03.toByte(), 0x2B.toByte(), 0x65.toByte(),
            0x70.toByte(), 0x03.toByte(), 0x21.toByte(), 0x00.toByte()
        )
        return prefix + rawKey
    }

    fun runClientHandshake(
        inputStream: InputStream,
        outputStream: OutputStream,
        localPublicKey: ByteArray,
        localName: ByteArray,
        signingKey: PrivateKey
    ): HandshakeResult {
        val clientNonce = generateNonce()
        
        val helloPayload = ByteBuffer.allocate(NONCE_SIZE + 2 + localPublicKey.size + 2 + localName.size)
            .put(clientNonce)
            .putShort(localPublicKey.size.toShort())
            .put(localPublicKey)
            .putShort(localName.size.toShort())
            .put(localName)
            .array()
            
        writeMessage(outputStream, Protocol.IDENTITY_HELLO, helloPayload)
        
        val (msgType, challengePayload) = readMessage(inputStream)
        if (msgType != Protocol.IDENTITY_CHALLENGE) {
            throw Exception("Expected IDENTITY_CHALLENGE, got $msgType")
        }
        
        val serverNonce = challengePayload.copyOfRange(0, NONCE_SIZE)
        val peerPublicKey = challengePayload.copyOfRange(NONCE_SIZE + 32, challengePayload.size)
        
        val digest = MessageDigest.getInstance("SHA-256")
        val peerHash = digest.digest(peerPublicKey)
        val clientHash = digest.digest(localPublicKey)
        
        val bytesToSign = ByteBuffer.allocate(NONCE_SIZE * 2 + 32 * 2)
            .put(serverNonce)
            .put(clientNonce)
            .put(peerHash)
            .put(clientHash)
            .array()
            
        val sig = try {
            Signature.getInstance("Ed25519", org.bouncycastle.jce.provider.BouncyCastleProvider())
        } catch (e: Exception) {
            Signature.getInstance("Ed25519")
        }
        sig.initSign(signingKey)
        sig.update(bytesToSign)
        val signatureBytes = sig.sign()
        
        writeMessage(outputStream, Protocol.IDENTITY_PROOF, signatureBytes)
        
        val (acceptMsgType, _) = readMessage(inputStream)
        if (acceptMsgType != Protocol.ACCEPT) {
            throw Exception("Server rejected handshake")
        }
        
        val sessionKey = CryptoUtils.deriveSessionKey(serverNonce, clientNonce, localPublicKey, peerPublicKey)
        return HandshakeResult(sessionKey, peerPublicKey, "Host Device")
    }

    fun runServerHandshake(
        inputStream: InputStream,
        outputStream: OutputStream,
        localPublicKey: ByteArray,
        localName: ByteArray
    ): HandshakeResult {
        val (helloType, helloPayload) = readMessage(inputStream)
        if (helloType != Protocol.IDENTITY_HELLO) throw Exception("Expected IDENTITY_HELLO")

        val clientNonce = helloPayload.copyOfRange(0, 32)
        var offset = 32
        val pkLen = ByteBuffer.wrap(helloPayload, offset, 2).short.toInt()
        offset += 2
        val peerPublicKey = helloPayload.copyOfRange(offset, offset + pkLen)
        offset += pkLen
        val nameLen = ByteBuffer.wrap(helloPayload, offset, 2).short.toInt()
        offset += 2
        val peerName = String(helloPayload, offset, nameLen, Charsets.UTF_8)
        
        val serverNonce = generateNonce()
        val digest = MessageDigest.getInstance("SHA-256")
        val peerHash = digest.digest(peerPublicKey)
        
        val challengePayload = ByteBuffer.allocate(32 + 32 + localPublicKey.size)
                .put(serverNonce)
                .put(peerHash)
                .put(localPublicKey)
                .array()
        writeMessage(outputStream, Protocol.IDENTITY_CHALLENGE, challengePayload)
        
        val (proofType, proofPayload) = readMessage(inputStream)
        if (proofType != Protocol.IDENTITY_PROOF) throw Exception("Expected IDENTITY_PROOF")

        val localHash = digest.digest(localPublicKey)
        val verifyBytes = ByteBuffer.allocate(32 + 32 + 32 + 32)
             .put(serverNonce)
             .put(clientNonce)
             .put(localHash)
             .put(peerHash)
             .array()
             
        val sig = try {
            Signature.getInstance("Ed25519", org.bouncycastle.jce.provider.BouncyCastleProvider())
        } catch (e: Exception) {
            Signature.getInstance("Ed25519")
        }
        val kf = try {
            KeyFactory.getInstance("Ed25519", org.bouncycastle.jce.provider.BouncyCastleProvider())
        } catch (e: Exception) {
            KeyFactory.getInstance("Ed25519")
        }
        val pubKey = kf.generatePublic(X509EncodedKeySpec(addEd25519Prefix(peerPublicKey)))
        
        sig.initVerify(pubKey)
        sig.update(verifyBytes)
        
        if (!sig.verify(proofPayload)) {
            writeMessage(outputStream, Protocol.REJECT, ByteArray(0))
            throw Exception("Invalid handshake signature")
        }
        
        writeMessage(outputStream, Protocol.ACCEPT, byteArrayOf(0))
        
        val sessionKey = CryptoUtils.deriveSessionKey(serverNonce, clientNonce, localPublicKey, peerPublicKey)
        return HandshakeResult(sessionKey, peerPublicKey, peerName)
    }

    private fun writeMessage(outputStream: OutputStream, msgType: Byte, payload: ByteArray) {
        val header = Protocol.makeHeader(msgType, payload.size.toLong())
        outputStream.write(header)
        outputStream.write(payload)
        outputStream.flush()
    }

    private fun readMessage(inputStream: InputStream): Pair<Byte, ByteArray> {
        val header = ByteArray(Protocol.HEADER_SIZE)
        var readHeader = 0
        while (readHeader < header.size) {
            val count = inputStream.read(header, readHeader, header.size - readHeader)
            if (count == -1) throw Exception("Unexpected EOF reading header")
            readHeader += count
        }
        val (msgType, payloadLen) = Protocol.parseHeader(header)
        val payload = ByteArray(payloadLen.toInt())
        var readPayload = 0
        while (readPayload < payload.size) {
            val count = inputStream.read(payload, readPayload, payload.size - readPayload)
            if (count == -1) throw Exception("Unexpected EOF reading payload")
            readPayload += count
        }
        return Pair(msgType, payload)
    }
}
