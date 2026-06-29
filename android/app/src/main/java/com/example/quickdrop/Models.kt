package com.example.quickdrop

data class IncomingOffer(
    val id: String,
    val fileName: String,
    val fileSize: Long,
    val peerName: String,
    val peerIp: String,
    val peerPublicKey: ByteArray?
) {
    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (javaClass != other?.javaClass) return false
        other as IncomingOffer
        return id == other.id
    }
    override fun hashCode(): Int = id.hashCode()
}

enum class TransferDirection {
    SEND, RECEIVE
}

enum class TransferStatus {
    ACTIVE, DONE, ERROR, PENDING, REJECTED, CANCELLED
}

data class TransferRecord(
    val id: String,
    val direction: TransferDirection,
    val fileName: String,
    val fileSize: Long,
    val bytesDone: Long,
    val status: TransferStatus,
    val peerName: String,
    val peerIp: String
)

data class DiscoveredPeer(
    val name: String,
    val ip: String,
    val port: Int
)

data class SelectedFile(
    val uri: android.net.Uri,
    val name: String,
    val size: Long
)
