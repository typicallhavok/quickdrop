package com.example.quickdrop

import android.content.Context

/**
 * Lightweight persisted app preferences (separate from the cryptographic
 * identity prefs). Currently holds the "resume interrupted transfers" toggle.
 */
object AppSettings {
    private const val PREFS = "app_prefs"
    private const val KEY_RESUME = "resume_transfers"

    private fun prefs(context: Context) =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /** Resume interrupted transfers from their partial file. Defaults to true. */
    fun resumeTransfers(context: Context): Boolean =
        prefs(context).getBoolean(KEY_RESUME, true)

    fun setResumeTransfers(context: Context, value: Boolean) {
        prefs(context).edit().putBoolean(KEY_RESUME, value).apply()
    }
}
