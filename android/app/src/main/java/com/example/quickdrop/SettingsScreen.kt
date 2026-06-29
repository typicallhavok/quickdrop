package com.example.quickdrop

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Phone
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(onClose: () -> Unit) {
    val context = LocalContext.current
    val trustManager = remember { TrustManager(context) }
    var trustedDevices by remember { mutableStateOf(trustManager.getAllTrustedDevices()) }
    var resumeTransfers by remember { mutableStateOf(AppSettings.resumeTransfers(context)) }
    
    val darkBg = MaterialTheme.colorScheme.background
    val cyan = MaterialTheme.colorScheme.primaryContainer
    val lightText = MaterialTheme.colorScheme.onSurface
    val iconGray = MaterialTheme.colorScheme.onSurfaceVariant
    val surfaceHigh = MaterialTheme.colorScheme.surfaceContainerHigh
    
    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(darkBg)
    ) {
        // App Bar
        TopAppBar(
            title = { Text("Settings", color = lightText, fontSize = 20.sp, fontWeight = FontWeight.Bold) },
            navigationIcon = {
                IconButton(onClick = onClose) {
                    Icon(Icons.Default.ArrowBack, contentDescription = "Back", tint = iconGray)
                }
            },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = darkBg)
        )
        
        LazyColumn(
            modifier = Modifier.fillMaxSize().padding(16.dp),
            contentPadding = PaddingValues(bottom = 32.dp)
        ) {
            item {
                Text(
                    "STORAGE",
                    color = cyan,
                    fontSize = 12.sp,
                    fontWeight = FontWeight.Bold,
                    modifier = Modifier.padding(bottom = 8.dp)
                )
                Card(
                    modifier = Modifier.fillMaxWidth().padding(bottom = 24.dp),
                    colors = CardDefaults.cardColors(containerColor = surfaceHigh)
                ) {
                    Column(modifier = Modifier.padding(16.dp)) {
                        Text("Download Folder", color = lightText, fontWeight = FontWeight.SemiBold, fontSize = 16.sp)
                        Text("Downloads/quickdrop", color = iconGray, fontSize = 14.sp, modifier = Modifier.padding(top = 4.dp))
                    }
                }
            }
            
            item {
                Text(
                    "TRANSFERS",
                    color = cyan,
                    fontSize = 12.sp,
                    fontWeight = FontWeight.Bold,
                    modifier = Modifier.padding(bottom = 8.dp)
                )
                Card(
                    modifier = Modifier.fillMaxWidth().padding(bottom = 24.dp),
                    colors = CardDefaults.cardColors(containerColor = surfaceHigh)
                ) {
                    Row(
                        modifier = Modifier.fillMaxWidth().padding(16.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Column(modifier = Modifier.weight(1f).padding(end = 12.dp)) {
                            Text("Resume Interrupted Transfers", color = lightText, fontWeight = FontWeight.SemiBold, fontSize = 16.sp)
                            Text(
                                "Continue a cancelled transfer where it left off. When off, a new file with a (n) suffix is created instead.",
                                color = iconGray,
                                fontSize = 14.sp,
                                modifier = Modifier.padding(top = 4.dp)
                            )
                        }
                        Switch(
                            checked = resumeTransfers,
                            onCheckedChange = {
                                resumeTransfers = it
                                AppSettings.setResumeTransfers(context, it)
                            }
                        )
                    }
                }
            }

            item {
                Text(
                    "TRUSTED DEVICES",
                    color = cyan,
                    fontSize = 12.sp,
                    fontWeight = FontWeight.Bold,
                    modifier = Modifier.padding(bottom = 8.dp)
                )
            }
            
            if (trustedDevices.isEmpty()) {
                item {
                    Column(
                        modifier = Modifier.fillMaxWidth().padding(vertical = 32.dp),
                        horizontalAlignment = Alignment.CenterHorizontally
                    ) {
                        Icon(Icons.Default.Lock, contentDescription = "Security", tint = iconGray.copy(alpha = 0.5f), modifier = Modifier.size(48.dp))
                        Spacer(modifier = Modifier.height(16.dp))
                        Text("You haven't trusted any devices yet.", color = iconGray, fontSize = 14.sp)
                    }
                }
            } else {
                items(trustedDevices) { device ->
                    Card(
                        modifier = Modifier.fillMaxWidth().padding(bottom = 8.dp),
                        colors = CardDefaults.cardColors(containerColor = surfaceHigh)
                    ) {
                        Row(
                            modifier = Modifier.fillMaxWidth().padding(16.dp),
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Icon(Icons.Default.Phone, contentDescription = "Device", tint = cyan)
                            Spacer(modifier = Modifier.width(16.dp))
                            Column(modifier = Modifier.weight(1f)) {
                                Text(device.second, color = lightText, fontWeight = FontWeight.SemiBold, fontSize = 16.sp)
                                Text(device.first.take(16) + "...", color = iconGray, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(top = 2.dp))
                            }
                            Spacer(modifier = Modifier.width(16.dp))
                            TextButton(
                                onClick = { 
                                    trustManager.removeDevice(device.first)
                                    trustedDevices = trustManager.getAllTrustedDevices()
                                },
                            ) {
                                Text("REVOKE", color = MaterialTheme.colorScheme.error, fontWeight = FontWeight.Bold, fontSize = 12.sp)
                            }
                        }
                    }
                }
            }
        }
    }
}
