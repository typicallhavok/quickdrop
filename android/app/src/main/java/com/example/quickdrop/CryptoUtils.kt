package com.example.quickdrop

import java.nio.ByteBuffer
import java.security.MessageDigest
import javax.crypto.Cipher
import javax.crypto.Mac
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

object CryptoUtils {

    fun deriveSessionKey(
        serverNonce: ByteArray,
        clientNonce: ByteArray,
        ourPub: ByteArray,
        peerPub: ByteArray
    ): ByteArray {
        val (pk1, pk2) = if (compareByteArrays(ourPub, peerPub) <= 0) {
            Pair(ourPub, peerPub)
        } else {
            Pair(peerPub, ourPub)
        }

        val ikm = ByteBuffer.allocate(32 + 32 + 32 + 32)
            .put(serverNonce)
            .put(clientNonce)
            .put(pk1)
            .put(pk2)
            .array()

        return hkdfSha256(ikm, "fastshare-session".toByteArray())
    }

    private fun hkdfSha256(ikm: ByteArray, info: ByteArray): ByteArray {
        // Simplified HKDF using HMAC-SHA256
        val mac = Mac.getInstance("HmacSHA256")
        // Since salt is None in rust (HKDF::<Sha256>::new(None, &ikm)), we use empty salt or 0s
        val salt = ByteArray(32)
        mac.init(SecretKeySpec(salt, "HmacSHA256"))
        val prk = mac.doFinal(ikm)
        
        mac.init(SecretKeySpec(prk, "HmacSHA256"))
        mac.update(info)
        mac.update(byteArrayOf(1))
        return mac.doFinal()
    }

    fun encrypt(key: ByteArray, counter: Long, plaintext: ByteArray): ByteArray {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        val secretKey = SecretKeySpec(key, "AES")
        val nonce = ByteBuffer.allocate(12)
            .put(ByteArray(4))
            .putLong(counter)
            .array()
        val gcmSpec = GCMParameterSpec(128, nonce)
        cipher.init(Cipher.ENCRYPT_MODE, secretKey, gcmSpec)
        return cipher.doFinal(plaintext)
    }

    fun decrypt(key: ByteArray, counter: Long, data: ByteArray): ByteArray {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        val secretKey = SecretKeySpec(key, "AES")
        val nonce = ByteBuffer.allocate(12)
            .put(ByteArray(4))
            .putLong(counter)
            .array()
        val gcmSpec = GCMParameterSpec(128, nonce)
        cipher.init(Cipher.DECRYPT_MODE, secretKey, gcmSpec)
        return cipher.doFinal(data)
    }

    private fun compareByteArrays(a: ByteArray, b: ByteArray): Int {
        for (i in 0 until minOf(a.size, b.size)) {
            val unsignedA = a[i].toInt() and 0xFF
            val unsignedB = b[i].toInt() and 0xFF
            if (unsignedA != unsignedB) return unsignedA.compareTo(unsignedB)
        }
        return a.size.compareTo(b.size)
    }
}
