<script setup lang="ts">
import { ref, computed, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Progress } from '@/components/ui/progress';
import { useDownloadQueue } from '@/composables/useDownloadQueue';
import { useProducts } from '@/composables/useProducts';

const props = defineProps<{
  productName: string;
}>();

const { products, isOffline, refreshData, pendingUninstallFor, setPendingUninstall } = useProducts();
const downloadQueue = useDownloadQueue();

const productData = computed(() => products.value[props.productName] ?? null);

// These computed properties reactively track the queue state for this product
const isActive = computed(() => downloadQueue.isActiveFor(props.productName));
const isQueued = computed(() => downloadQueue.isQueuedFor(props.productName));
const queuedItem = computed(() => downloadQueue.getQueuedItem(props.productName));
const activeItem = computed(() => downloadQueue.activeItem.value);
const progress = computed(() => downloadQueue.progress.value);

// Version selector for fresh installs, reset when product changes or data refreshes
const targetInstallVersion = ref('');
watch(productData, (newData) => {
  if (newData?.latest_version) {
    targetInstallVersion.value = newData.latest_version;
  }
}, { immediate: true });

// Uninstall has its own separate busy state (not part of the queue)
const isUninstalling = ref(false);
const isBusy = computed(() => isActive.value || isQueued.value || isUninstalling.value);

const taskLabel: Record<string, string> = {
  install: 'Installing',
  update: 'Updating',
  repair: 'Repairing',
};

function queueInstallOrUpdate() {
  if (!productData.value) return;
  downloadQueue.addToQueue({
    productName: props.productName,
    targetVersion: targetInstallVersion.value || productData.value.latest_version,
    availableVersions: productData.value.versions,
    type: productData.value.local_version ? 'update' : 'install',
  });
}

function queueRepair() {
  if (!productData.value?.local_version) return;
  downloadQueue.addToQueue({
    productName: props.productName,
    targetVersion: productData.value.local_version,
    availableVersions: productData.value.versions,
    type: 'repair',
  });
}

async function launchApp() {
  try {
    await invoke('launch_product', { productName: props.productName });
  } catch (err: any) {
    alert(`Failed to launch: ${err}`);
  }
}

async function uninstallProduct() {
  if (!confirm(`Are you sure you want to uninstall ${props.productName}?`)) return;

  isUninstalling.value = true;
  let success = false;

  while (!success) {
    try {
      await invoke('uninstall_product', { productName: props.productName });
      await refreshData();
      success = true;
    } catch (err: any) {
      const errorString = String(err);

      if (errorString.includes('currently running')) {
        if (confirm(`${props.productName} is currently running.\n\nDo you want to force close it to continue uninstalling?`)) {
          try {
            await invoke('force_kill_product', { productName: props.productName });
            await new Promise(r => setTimeout(r, 3000));
          } catch {
            alert(`Could not close the application automatically. Please close it manually.`);
            break;
          }
        } else {
          break;
        }
      } else {
        alert(`Failed to uninstall: ${errorString}`);
        break;
      }
    }
  }

  isUninstalling.value = false;
}

// Watch for uninstall intent triggered from outside (e.g. Windows "Add/Remove Programs")
watch(pendingUninstallFor, async (intent) => {
  if (intent === props.productName) {
    setPendingUninstall(null);
    await uninstallProduct();
  }
});
</script>

<template>
  <Card v-if="productData" class="max-w-3xl mx-auto shadow-lg">
    <CardHeader>
      <CardTitle class="text-3xl">{{ productName }}</CardTitle>
      <CardDescription>Manage your installation and updates.</CardDescription>
    </CardHeader>

    <CardContent>
      <!-- Version info -->
      <div class="flex gap-8 mb-6 text-sm">
        <div class="flex flex-col">
          <span class="text-muted-foreground">Local Version</span>
          <span class="font-mono font-medium text-lg">
            {{ productData.local_version || 'Not Installed' }}
          </span>
        </div>
        <div class="flex flex-col">
          <span class="text-muted-foreground">Latest Version</span>
          <span class="font-mono font-medium text-lg text-primary">
            {{ productData.latest_version }}
          </span>
        </div>
      </div>

      <div class="space-y-4 pt-6 border-t border-border">

        <!-- Queued status banner -->
        <div
          v-if="isQueued"
          class="bg-yellow-900/20 border border-yellow-700 p-3 rounded-lg flex items-center justify-between"
        >
          <span class="text-sm text-yellow-300">
            Queued for {{ taskLabel[queuedItem!.type] ?? queuedItem!.type }}…
          </span>
          <Button
            variant="ghost"
            size="sm"
            class="text-yellow-400 hover:text-yellow-200 h-7"
            @click="downloadQueue.removeFromQueue(productName)"
          >
            Remove from queue
          </Button>
        </div>

        <!-- Fresh install section -->
        <div v-if="!productData.local_version" class="flex items-center gap-4">
          <Select v-model="targetInstallVersion" :disabled="isBusy || isOffline">
            <SelectTrigger class="w-45">
              <SelectValue placeholder="Select version" />
            </SelectTrigger>
            <SelectContent>
              <SelectGroup>
                <SelectItem v-for="v in productData.versions" :key="v" :value="v">
                  Install v{{ v }}
                </SelectItem>
              </SelectGroup>
            </SelectContent>
          </Select>

          <Button @click="queueInstallOrUpdate" :disabled="isBusy || isOffline" size="lg">
            {{ isQueued ? 'Queued…' : 'Install Product' }}
          </Button>
        </div>

        <!-- Installed section -->
        <div v-if="productData.local_version" class="space-y-6">

          <!-- Update available banner -->
          <div
            v-if="productData.local_version !== productData.latest_version"
            class="bg-blue-900/20 border border-blue-800 p-4 rounded-lg flex items-center justify-between"
          >
            <div>
              <h4 class="font-bold text-blue-400">Update Available!</h4>
              <p class="text-sm text-blue-200">
                Version {{ productData.latest_version }} is ready to install.
              </p>
            </div>
            <Button
              @click="queueInstallOrUpdate"
              :disabled="isBusy || isOffline"
              class="bg-blue-600 hover:bg-blue-500 text-white"
            >
              {{ isQueued ? 'Queued' : 'Update Now' }}
            </Button>
          </div>

          <!-- Up to date -->
          <div v-else class="text-green-500 font-bold flex items-center gap-2">
            <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/>
              <polyline points="22 4 12 14.01 9 11.01"/>
            </svg>
            Product is fully up to date!
          </div>

          <!-- Action buttons -->
          <div class="flex gap-4 w-full">
            <Button
              @click="launchApp"
              :disabled="isUninstalling"
              size="lg"
              class="flex-1 text-lg h-12"
            >
              Launch Product
            </Button>
            <Button
              variant="secondary"
              @click="queueRepair"
              :disabled="isBusy || isOffline"
              size="lg"
              class="h-12"
            >
              Repair
            </Button>
            <Button
              variant="destructive"
              @click="uninstallProduct"
              :disabled="isBusy"
              size="lg"
              class="h-12"
            >
              {{ isUninstalling ? 'Uninstalling…' : 'Uninstall' }}
            </Button>
          </div>
        </div>

        <!-- Active progress for THIS product only -->
        <div
          v-if="isActive"
          class="pt-6 space-y-2 animate-in fade-in slide-in-from-bottom-2"
        >
          <div class="flex justify-between items-center">
            <span class="text-sm font-medium text-muted-foreground">
              {{ taskLabel[activeItem!.type] ?? activeItem!.type }}…
            </span>
            <div class="flex items-center gap-3">
              <span class="text-xs text-muted-foreground">
                {{ progress.percent.toFixed(0) }}%
              </span>
              <!-- Cancel only available for fresh installs -->
              <Button
                v-if="activeItem!.type === 'install'"
                variant="destructive"
                size="sm"
                class="h-7 px-3 text-xs"
                @click="downloadQueue.cancelCurrent()"
              >
                Stop Installation
              </Button>
            </div>
          </div>
          <Progress
            :model-value="progress.percent"
            class="h-2 w-full"
            :class="{ 'animate-pulse': progress.percent === 0 }"
          />
        </div>

      </div>
    </CardContent>
  </Card>
</template>


