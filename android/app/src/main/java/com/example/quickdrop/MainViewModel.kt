package com.example.quickdrop

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class TrustPrompt(
    val peerPublicKey: ByteArray,
    val peerName: String
)

class MainViewModel : ViewModel() {

    private val _incomingOffers = MutableStateFlow<List<IncomingOffer>>(emptyList())
    val incomingOffers: StateFlow<List<IncomingOffer>> = _incomingOffers.asStateFlow()

    private val _transfers = MutableStateFlow<List<TransferRecord>>(emptyList())
    val transfers: StateFlow<List<TransferRecord>> = _transfers.asStateFlow()

    private val _isConnected = MutableStateFlow(false)
    val isConnected: StateFlow<Boolean> = _isConnected.asStateFlow()

    private val _connectedPeer = MutableStateFlow<String?>("Not connected")
    val connectedPeer: StateFlow<String?> = _connectedPeer.asStateFlow()

    private val _discoveredPeers = MutableStateFlow<List<DiscoveredPeer>>(emptyList())
    val discoveredPeers: StateFlow<List<DiscoveredPeer>> = _discoveredPeers.asStateFlow()

    private val _selectedFiles = MutableStateFlow<List<SelectedFile>>(emptyList())
    val selectedFiles: StateFlow<List<SelectedFile>> = _selectedFiles.asStateFlow()

    private val _trustPrompt = MutableStateFlow<TrustPrompt?>(null)
    val trustPrompt: StateFlow<TrustPrompt?> = _trustPrompt.asStateFlow()
    
    var onTrustDecision: ((Boolean) -> Unit)? = null

    fun addDiscoveredPeer(peer: DiscoveredPeer) {
        if (_discoveredPeers.value.none { it.ip == peer.ip }) {
            _discoveredPeers.update { it + peer }
        }
    }

    fun setConnectionState(connected: Boolean, peer: String? = "Not connected") {
        _isConnected.value = connected
        _connectedPeer.value = peer
    }
    
    fun requestTrustDecision(prompt: TrustPrompt, callback: (Boolean) -> Unit) {
        onTrustDecision = callback
        _trustPrompt.value = prompt
    }
    
    fun addSelectedFile(file: SelectedFile) {
        _selectedFiles.update { it + file }
    }

    fun removeSelectedFile(uri: android.net.Uri) {
        _selectedFiles.update { list -> list.filter { it.uri != uri } }
    }
    
    fun clearSelectedFiles() {
        _selectedFiles.value = emptyList()
    }

    fun clearSelectedFile(name: String) {
        _selectedFiles.update { list -> list.filter { it.name != name } }
    }

    fun submitTrustDecision(isTrusted: Boolean) {
        val callback = onTrustDecision
        onTrustDecision = null
        _trustPrompt.value = null
        callback?.invoke(isTrusted)
    }

    fun addOutgoingTransfer(id: String, fileName: String, fileSize: Long, peerName: String, peerIp: String) {
        val transfer = TransferRecord(
            id = id,
            direction = TransferDirection.SEND,
            fileName = fileName,
            fileSize = fileSize,
            bytesDone = 0L,
            status = TransferStatus.PENDING,
            peerName = peerName,
            peerIp = peerIp
        )
        _transfers.update { it + transfer }
    }

    fun markTransferRejected(fileName: String) {
        _transfers.update { list ->
            list.map {
                if (it.fileName == fileName && it.status == TransferStatus.PENDING) {
                    it.copy(status = TransferStatus.REJECTED)
                } else it
            }
        }
        scheduleTerminalRemoval()
    }

    fun markTransferError(fileName: String) {
        _transfers.update { list ->
            list.map {
                if (it.fileName == fileName && (it.status == TransferStatus.ACTIVE || it.status == TransferStatus.PENDING)) {
                    it.copy(status = TransferStatus.ERROR)
                } else it
            }
        }
        scheduleTerminalRemoval()
    }
    
    fun markTransferActive(fileName: String) {
        _transfers.update { list ->
            list.map {
                if (it.fileName == fileName && it.status == TransferStatus.PENDING) {
                    it.copy(status = TransferStatus.ACTIVE)
                } else it
            }
        }
    }

    // Add incoming file offer manually (when untrusted)
    fun addOffer(offer: IncomingOffer) {
        _incomingOffers.update { it + offer }
    }

    // This bypasses the Inbox UI and maps it directly to Activity list (when Trusted)
    fun autoAcceptOffer(offerId: String, fileName: String, fileSize: Long, peerName: String, peerIp: String) {
        val newTransfer = TransferRecord(
            id = offerId,
            direction = TransferDirection.RECEIVE,
            fileName = fileName,
            fileSize = fileSize,
            bytesDone = 0L,
            status = TransferStatus.ACTIVE,
            peerName = peerName,
            peerIp = peerIp
        )
        _transfers.update { it + newTransfer }
    }

    fun acceptOffer(offerId: String) {
        val offer = _incomingOffers.value.find { it.id == offerId } ?: return
        _incomingOffers.update { it.filter { o -> o.id != offerId } }
        
        val newTransfer = TransferRecord(
            id = offerId,
            direction = TransferDirection.RECEIVE,
            fileName = offer.fileName,
            fileSize = offer.fileSize,
            bytesDone = 0L,
            status = TransferStatus.ACTIVE,
            peerName = offer.peerName,
            peerIp = offer.peerIp
        )
        _transfers.update { it + newTransfer }
    }

    fun rejectOffer(offerId: String) {
        _incomingOffers.update { it.filter { o -> o.id != offerId } }
    }

    fun cancelTransfer(id: String) {
        _transfers.update { list ->
            list.map {
                if (it.id == id && (it.status == TransferStatus.ACTIVE || it.status == TransferStatus.PENDING)) {
                    it.copy(status = TransferStatus.CANCELLED)
                } else it
            }
        }
        scheduleTerminalRemoval()
    }

    /** Terminal transfers (done/error/cancelled/rejected) shouldn't linger in the
     *  list — remove each one ~2s after it reaches a terminal state. Idempotent:
     *  a given id is only scheduled once. */
    private val scheduledForRemoval = java.util.Collections.synchronizedSet(mutableSetOf<String>())

    private fun scheduleTerminalRemoval() {
        val terminal = setOf(
            TransferStatus.DONE,
            TransferStatus.ERROR,
            TransferStatus.CANCELLED,
            TransferStatus.REJECTED
        )
        for (t in _transfers.value) {
            if (t.status in terminal && scheduledForRemoval.add(t.id)) {
                viewModelScope.launch {
                    delay(2000)
                    _transfers.update { list -> list.filter { it.id != t.id } }
                    scheduledForRemoval.remove(t.id)
                }
            }
        }
    }

    fun updateTransferProgress(fileName: String, bytesDone: Long) {
        _transfers.update { list ->
            list.map {
                if (it.fileName == fileName && it.status == TransferStatus.ACTIVE) {
                    it.copy(bytesDone = bytesDone)
                } else it
            }
        }
    }

    fun markTransferDone(fileName: String) {
        _transfers.update { list ->
            list.map {
                if (it.fileName == fileName && it.status == TransferStatus.ACTIVE) {
                    it.copy(status = TransferStatus.DONE, bytesDone = it.fileSize)
                } else it
            }
        }
        scheduleTerminalRemoval()
    }
}
