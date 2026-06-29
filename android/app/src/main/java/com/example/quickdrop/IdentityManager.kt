package com.example.quickdrop

import android.content.Context
import android.content.SharedPreferences
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyFactory
import java.security.KeyPair
import java.security.KeyPairGenerator
import java.security.PrivateKey
import java.security.PublicKey
import java.security.spec.PKCS8EncodedKeySpec
import java.security.spec.X509EncodedKeySpec

class IdentityManager(context: Context) {

    private val prefs: SharedPreferences = context.getSharedPreferences("identity_prefs", Context.MODE_PRIVATE)

    var keyPair: KeyPair
        private set
        
    var localName: String
        private set

    init {
        // Try to load existing keys, or generate new ones
        val privKeyBase64 = prefs.getString("private_key", null)
        val pubKeyBase64 = prefs.getString("public_key", null)
        val savedName = prefs.getString("local_name", null)

        if (privKeyBase64 != null && pubKeyBase64 != null && savedName != null) {
            val kf = try {
                val bcProvider = org.bouncycastle.jce.provider.BouncyCastleProvider()
                KeyFactory.getInstance("Ed25519", bcProvider)
            } catch (e: Exception) {
                KeyFactory.getInstance("Ed25519") // fallback
            }
            val privKeySpec = PKCS8EncodedKeySpec(Base64.decode(privKeyBase64, Base64.DEFAULT))
            val pubKeySpec = X509EncodedKeySpec(Base64.decode(pubKeyBase64, Base64.DEFAULT))
            
            val privateKey: PrivateKey = kf.generatePrivate(privKeySpec)
            val publicKey: PublicKey = kf.generatePublic(pubKeySpec)
            
            keyPair = KeyPair(publicKey, privateKey)
            localName = savedName
        } else {
            // Find a software provider for Ed25519 to allow exporting the private key
            val kpg: KeyPairGenerator = try {
                val bcProvider = org.bouncycastle.jce.provider.BouncyCastleProvider()
                KeyPairGenerator.getInstance("Ed25519", bcProvider)
            } catch (e: Exception) {
                KeyPairGenerator.getInstance("Ed25519") // fallback
            }
            
            // Software providers typically don't need AndroidKeyStore initialization parameters
            keyPair = kpg.generateKeyPair()
            
            localName = android.os.Build.MODEL ?: "Android Device"
            
            // Save them
            prefs.edit()
                .putString("private_key", Base64.encodeToString(keyPair.private.encoded, Base64.DEFAULT))
                .putString("public_key", Base64.encodeToString(keyPair.public.encoded, Base64.DEFAULT))
                .putString("local_name", localName)
                .apply()
        }
    }

    fun getPublicKeyBytes(): ByteArray {
        // X.509 encoded Ed25519 keys in Java have a 12-byte ASN.1 prefix. 
        // We typically need just the 32-byte raw key for the rust backend.
        // The last 32 bytes of the X.509 encoding are the raw public key.
        val encoded = keyPair.public.encoded
        return if (encoded.size > 32) {
            encoded.copyOfRange(encoded.size - 32, encoded.size)
        } else {
            encoded
        }
    }
}
