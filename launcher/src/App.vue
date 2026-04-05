<script setup lang="ts">
import { onMounted } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useProducts } from '@/composables/useProducts';
import { useDownloadQueue } from '@/composables/useDownloadQueue';
import ProductSidebar from '@/components/ProductSidebar.vue';
import ProductDetail from '@/components/ProductDetail.vue';

const {
  products,
  isOffline,
  selectedProductName,
  loadCache,
  refreshData,
  selectProduct,
  setPendingUninstall,
} = useProducts();

const downloadQueue = useDownloadQueue();

async function processUninstallIntent(intent: string | null) {
  if (intent && products.value[intent]) {
    selectProduct(intent);
    setPendingUninstall(intent);
  }
}

onMounted(async () => {
  // Initialize progress listener for the download queue
  await downloadQueue.init();

  // Wire up the refresh callback so the queue can update product state after each operation
  downloadQueue.setRefreshCallback(refreshData);

  // Load local cache immediately for a fast first render
  await loadCache();

  // Listen for uninstall intents from the single-instance plugin
  await listen<string>('uninstall-intent', async (event) => {
    await processUninstallIntent(event.payload);
  });

  // Fetch fresh data from the server
  await refreshData();

  // Handle startup uninstall intent (cold start avoids race condition with the event listener)
  try {
    const intent: string | null = await invoke('get_startup_intent');
    await processUninstallIntent(intent);
  } catch {
    // No startup intent
  }
});
</script>

<template>
  <div class="flex flex-col h-screen w-screen bg-background text-foreground font-sans overflow-hidden">

    <!-- Offline banner -->
    <div
      v-if="isOffline"
      class="bg-destructive text-destructive-foreground text-center py-2 text-sm font-bold w-full z-50 shadow-md"
    >
      Offline Mode: Server unreachable. You can only launch or uninstall existing applications.
    </div>

    <div class="flex flex-1 overflow-hidden">
      <ProductSidebar />

      <main class="flex-1 p-8 overflow-y-auto bg-muted/20">
        <ProductDetail
          v-if="selectedProductName"
          :product-name="selectedProductName"
        />
        <div v-else class="flex h-full items-center justify-center text-muted-foreground">
          Select a product from the menu to manage it.
        </div>
      </main>
    </div>

  </div>
</template>

