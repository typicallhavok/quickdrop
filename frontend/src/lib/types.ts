export interface Settings {
  local_name: string;
  download_dir: string;
  run_in_tray: boolean;
  /** Resume interrupted transfers from their partial file. When false, a fresh
   *  file is received instead (saved with a " (n)" suffix). */
  resume_transfers: boolean;
  port?: number;
}

export interface Transfer {
  id: string;
  file_name: string;
  file_size: number;
  bytes_done: number;
  direction: 'send' | 'receive';
  peer_name: string;
  peer_ip: string;
  status: 'pending' | 'active' | 'done' | 'error' | 'rejected' | 'cancelled';
  speed_bps?: number;
}

export interface IncomingOffer {
  id: string;
  file_name: string;
  file_size: number;
  peer_name: string;
  peer_ip: string;
  is_trusted: boolean;
}

export interface Device {
  id: string;
  name: string;
  public_key: string;
}

export interface DiscoveredDevice {
  id: string;
  name: string;
}
