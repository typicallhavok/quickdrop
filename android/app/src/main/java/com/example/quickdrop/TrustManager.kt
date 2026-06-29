package com.example.quickdrop

import android.content.Context
import android.content.SharedPreferences
import android.util.Base64

enum class TrustState {
    UNKNOWN, TRUSTED, UNTRUSTED
}

class TrustManager(context: Context) {
    private val prefs: SharedPreferences = context.getSharedPreferences("trust_prefs", Context.MODE_PRIVATE)

    fun getTrustState(pubKey: ByteArray): TrustState {
        val keyStr = Base64.encodeToString(pubKey, Base64.NO_WRAP)
        val state = prefs.getString(keyStr, null)
        if (state == null) return TrustState.UNKNOWN
        if (state.startsWith("TRUSTED")) return TrustState.TRUSTED
        if (state.startsWith("UNTRUSTED")) return TrustState.UNTRUSTED
        return TrustState.UNKNOWN
    }

    fun setTrustState(pubKey: ByteArray, isTrusted: Boolean, peerName: String = "Unknown Device") {
        val keyStr = Base64.encodeToString(pubKey, Base64.NO_WRAP)
        prefs.edit().putString(keyStr, if (isTrusted) "TRUSTED|$peerName" else "UNTRUSTED|$peerName").apply()
    }

    fun getAllTrustedDevices(): List<Pair<String, String>> {
        val trusted = mutableListOf<Pair<String, String>>()
        for ((key, value) in prefs.all) {
            val vStr = value as? String ?: continue
            if (vStr.startsWith("TRUSTED|")) {
                val name = vStr.substringAfter("|")
                trusted.add(key to name)
            } else if (vStr == "TRUSTED") {
                trusted.add(key to "Unknown Device")
            }
        }
        return trusted
    }

    fun removeDevice(base64PubKey: String) {
        prefs.edit().remove(base64PubKey).apply()
    }
}
