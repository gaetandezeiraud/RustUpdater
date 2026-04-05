import { ref, computed } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type TaskType = 'install' | 'update' | 'repair';

export interface QueueItem {
  productName: string;
  targetVersion: string;
  availableVersions: string[];
  type: TaskType;
}

interface ProgressData {
  current: number;
  total: number;
  percent: number;
  highest: number;
}

// --- Module-level singleton state (shared across all composable instances) ---
const _queue = ref<QueueItem[]>([]);
const _activeItem = ref<QueueItem | null>(null);
const _progress = ref<ProgressData>({ current: 0, total: 0, percent: 0, highest: 0 });
const _isProcessing = ref(false);

let _refreshCallback: (() => Promise<void>) | null = null;
let _listenerInitialized = false;

async function _initListener() {
  if (_listenerInitialized) return;
  _listenerInitialized = true;

  await listen<any>('progress', (event) => {
    const data = event.payload;
    // Only update progress for the currently active product
    if (!_activeItem.value || data.product_name !== _activeItem.value.productName) return;

    const actualPercent = data.total > 0 ? (data.current / data.total) * 100 : 0;

    // Prevent the bar from going backwards
    if (actualPercent > _progress.value.highest) {
      _progress.value.highest = actualPercent;
    } else if (actualPercent === 0) {
      _progress.value.highest = 0;
    }

    _progress.value = {
      current: data.current,
      total: data.total,
      percent: _progress.value.highest,
      highest: _progress.value.highest,
    };
  });
}

async function _processNext() {
  if (_isProcessing.value || _queue.value.length === 0) return;

  const item = _queue.value.shift()!;
  _activeItem.value = item;
  _isProcessing.value = true;
  _progress.value = { current: 0, total: 0, percent: 0, highest: 0 };

  let success = false;

  while (!success) {
    try {
      if (item.type === 'repair') {
        await invoke('repair_installation', {
          productName: item.productName,
          version: item.targetVersion,
        });
      } else {
        await invoke('run_update', {
          productName: item.productName,
          targetVersion: item.targetVersion,
          availableVersions: item.availableVersions,
        });
      }

      _progress.value.percent = 100;
      if (_refreshCallback) await _refreshCallback();
      success = true;
    } catch (err: any) {
      const errorString = String(err);

      if (errorString.includes('CANCELLED')) {
        // User cancelled, silently refresh and exit
        if (_refreshCallback) await _refreshCallback();
        break;
      } else if (errorString.includes('currently running')) {
        if (confirm(`${item.productName} is currently running.\n\nDo you want to force close it to continue?`)) {
          try {
            await invoke('force_kill_product', { productName: item.productName });
            await new Promise(r => setTimeout(r, 1000));
            // Loop again to retry
          } catch {
            alert(`Could not close the application automatically. Please close it manually.`);
            if (_refreshCallback) await _refreshCallback();
            break;
          }
        } else {
          if (_refreshCallback) await _refreshCallback();
          break;
        }
      } else if (errorString.includes('INSUFFICIENT_SPACE')) {
        const parts = errorString.split(':');
        const reqGb = (parseInt(parts[1]) / 1024 / 1024 / 1024).toFixed(2);
        const availGb = (parseInt(parts[2]) / 1024 / 1024 / 1024).toFixed(2);
        alert(`Not enough disk space for ${item.productName}!\nRequired: ${reqGb} GB - Available: ${availGb} GB`);
        if (_refreshCallback) await _refreshCallback();
        break;
      } else {
        alert(`Operation failed for ${item.productName}: ${errorString}`);
        if (_refreshCallback) await _refreshCallback();
        break;
      }
    }
  }

  _activeItem.value = null;
  _isProcessing.value = false;

  // Process the next item in the queue after a short delay
  if (_queue.value.length > 0) {
    setTimeout(_processNext, 400);
  }
}

export function useDownloadQueue() {
  return {
    queue: computed(() => _queue.value),
    activeItem: computed(() => _activeItem.value),
    progress: computed(() => _progress.value),
    isProcessing: computed(() => _isProcessing.value),

    isActiveFor(productName: string): boolean {
      return _activeItem.value?.productName === productName;
    },

    isQueuedFor(productName: string): boolean {
      return _queue.value.some(i => i.productName === productName);
    },

    getQueuedItem(productName: string): QueueItem | null {
      return _queue.value.find(i => i.productName === productName) ?? null;
    },

    async init() {
      await _initListener();
    },

    setRefreshCallback(cb: () => Promise<void>) {
      _refreshCallback = cb;
    },

    addToQueue(item: QueueItem) {
      // Don't add if already active or already in queue for this product
      if (_activeItem.value?.productName === item.productName) return;
      if (_queue.value.some(i => i.productName === item.productName)) return;
      _queue.value.push(item);
      _processNext();
    },

    removeFromQueue(productName: string) {
      const idx = _queue.value.findIndex(i => i.productName === productName);
      if (idx !== -1) _queue.value.splice(idx, 1);
    },

    async cancelCurrent() {
      if (!_activeItem.value) return;
      try {
        await invoke('cancel_update');
      } catch (err) {
        console.error('Failed to send cancel signal:', err);
      }
    },
  };
}

