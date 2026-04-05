import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';

// --- Module-level singleton state ---
const _products = ref<Record<string, any>>({});
const _isOffline = ref(false);
const _selectedProductName = ref('');
const _selectedProductData = ref<any>(null);
const _pendingUninstallFor = ref<string | null>(null);

export function useProducts() {
  async function refreshData() {
    try {
      const state: any = await invoke('get_app_state');
      _products.value = state.products || {};
      _isOffline.value = state.offline;

      // Re-sync selected product data after refresh
      if (_selectedProductName.value) {
        _selectedProductData.value = _products.value[_selectedProductName.value] ?? null;
      }
    } catch (err: any) {
      alert('Failed to fetch app state: ' + err);
    }
  }

  async function loadCache() {
    try {
      const cachedState: any = await invoke('get_cached_app_state');
      if (cachedState && Object.keys(cachedState.products).length > 0) {
        _products.value = cachedState.products;
        _isOffline.value = false;

        const firstProduct = Object.keys(_products.value)[0];
        if (firstProduct && !_selectedProductName.value) {
          selectProduct(firstProduct);
        }
      }
    } catch {
      console.warn('No local cache found yet.');
    }
  }

  function selectProduct(name: string) {
    _selectedProductName.value = name;
    _selectedProductData.value = _products.value[name] ?? null;
  }

  function setPendingUninstall(name: string | null) {
    _pendingUninstallFor.value = name;
  }

  return {
    products: _products,
    isOffline: _isOffline,
    selectedProductName: _selectedProductName,
    selectedProductData: _selectedProductData,
    pendingUninstallFor: _pendingUninstallFor,
    refreshData,
    loadCache,
    selectProduct,
    setPendingUninstall,
  };
}

