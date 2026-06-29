package com.example.quickdrop

import java.nio.ByteBuffer
import java.nio.ByteOrder

object Protocol {
    const val IDENTITY_HELLO: Byte = 0x10
    const val IDENTITY_CHALLENGE: Byte = 0x11
    const val IDENTITY_PROOF: Byte = 0x12
    const val ACCEPT: Byte = 0x13
    const val REJECT: Byte = 0x14
    const val HEADER_SIZE = 9

    const val FILE_BEGIN: Byte = 0x19
    const val FILE_UPLOAD: Byte = 0x20
    const val FILE_CHUNK: Byte = 0x21
    const val FILE_END: Byte = 0x22
    const val FILE_OFFER: Byte = 0x23
    const val OFFER_ACCEPT: Byte = 0x24
    const val OFFER_REJECT: Byte = 0x25

    /** Push clipboard text to the peer (UTF-8 payload). Receiver copies it into
     *  its system clipboard and notifies the user. */
    const val CLIPBOARD: Byte = 0x30

    /** Large TCP socket buffers so the window can open up on Wi-Fi / Wi-Fi Direct. */
    const val SOCKET_BUFFER_SIZE = 4 * 1024 * 1024

    /** Below this many already-received bytes, restart instead of resuming. */
    const val RESUME_MIN_BYTES = 8L * 1024 * 1024
    /** When resuming, rewind this many bytes and re-receive them (overwrites a torn tail). */
    const val RESUME_REWIND_BYTES = 1L * 1024 * 1024

    fun makeHeader(msgType: Byte, payloadLen: Long): ByteArray {
        val buffer = ByteBuffer.allocate(HEADER_SIZE)
        buffer.order(ByteOrder.BIG_ENDIAN)
        buffer.put(msgType)
        buffer.putLong(payloadLen)
        return buffer.array()
    }

    fun parseHeader(header: ByteArray): Pair<Byte, Long> {
        val buffer = ByteBuffer.wrap(header)
        buffer.order(ByteOrder.BIG_ENDIAN)
        val msgType = buffer.get()
        val payloadLen = buffer.long
        return Pair(msgType, payloadLen)
    }
}
