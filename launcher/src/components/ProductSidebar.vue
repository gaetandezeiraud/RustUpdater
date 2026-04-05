<script setup lang="ts">
import { useProducts } from '@/composables/useProducts';
import { useDownloadQueue } from '@/composables/useDownloadQueue';
import { Button } from '@/components/ui/button';
import DownloadManager from '@/components/DownloadManager.vue';

const { products, selectedProductName, selectProduct, refreshData } = useProducts();
const { isActiveFor, isQueuedFor } = useDownloadQueue();
</script>

<template>
  <aside class="w-64 bg-card flex flex-col border-r border-border shrink-0">
    <!-- Product list -->
    <div class="p-4 flex-1 overflow-y-auto">
      <h1 class="text-xl font-bold mb-6 text-primary">Launcher</h1>

      <div v-if="Object.keys(products).length === 0" class="text-muted-foreground text-sm">
        No products found.
      </div>

      <div class="flex flex-col gap-1.5">
        <Button
          v-for="name in Object.keys(products)"
          :key="name"
          :variant="selectedProductName === name ? 'default' : 'ghost'"
          class="justify-start w-full relative"
          @click="selectProduct(name)"
        >
          <span class="truncate">{{ name }}</span>
          <!-- Activity indicator dot -->
          <span
            v-if="isActiveFor(name)"
            class="ml-auto h-2 w-2 rounded-full bg-blue-400 animate-pulse shrink-0"
          />
          <span
            v-else-if="isQueuedFor(name)"
            class="ml-auto h-2 w-2 rounded-full bg-yellow-400 shrink-0"
          />
        </Button>
      </div>
    </div>

    <!-- Bottom panel: Refresh + Download Manager -->
    <div class="border-t border-border p-4 space-y-3">
      <Button variant="outline" class="w-full" @click="refreshData">
        Refresh Data
      </Button>
      <DownloadManager />
    </div>
  </aside>
</template>



