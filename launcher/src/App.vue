<script setup lang="ts">
import { ref, onMounted } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

// Shadcn Components
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { ScrollArea } from '@/components/ui/scroll-area';
import { Progress } from '@/components/ui/progress';

// State
const products = ref<Record<string, any>>({});
const selectedProductName = ref('');
const selectedProductData = ref<any>(null);
const targetInstallVersion = ref('');

const isOffline = ref(false);
const isBusy = ref(false);
const currentTaskName = ref('');
const progressData = ref({ current: 0, total: 0, percent: 0, highest: 0 });

const showLogsModal = ref(false);
const logs = ref<string[]>([]);

async function processUninstallIntent(intent: string | null) {
  if (intent && products.value[intent]) {
    await selectProduct(intent);
    uninstallProduct();
  }
}

onMounted(async () => {
  // Listen for logs emitted
  await listen<string>('log', (event) => {
    logs.value.push(event.payload);
  });

  // Listen for detailed progress emitted from Rust
  await listen<any>('progress', (event) => {
    const data = event.payload;

    // Calculate the true percentage
    let actualPercent = 0;
    if (data.total > 0) {
      actualPercent = (data.current / data.total) * 100;
    }

    // Prevent the progress bar from ever going backwards
    if (actualPercent > progressData.value.highest) {
      progressData.value.highest = actualPercent;
    } else if (actualPercent === 0) {
      progressData.value.highest = 0;
    }

    progressData.value.current = data.current;
    progressData.value.total = data.total;
    progressData.value.percent = progressData.value.highest;
  });

  // Load local cache
  try {
    const cachedState: any = await invoke('get_cached_app_state');
    if (cachedState && Object.keys(cachedState.products).length > 0) {
      products.value = cachedState.products;
      isOffline.value = false;

      const firstProduct = Object.keys(products.value)[0];
      if (firstProduct && !selectedProductName.value) {
        await selectProduct(firstProduct);
      }
    }
  } catch (err) {
    console.warn("No local cache found yet.");
  }

  // Listen for uninstall intents
  await listen<string>('uninstall-intent', async (event) => {
    processUninstallIntent(event.payload);
  });

  // Automatically fetch data on startup since Rust handles the URL now
  await refreshData();

  // Cold start, doesn't use the event to avoid race condition at first start-up
  try {
    const intent: string | null = await invoke('get_startup_intent');
    await processUninstallIntent(intent);
  } catch (err) {
    console.warn("Failed to check startup intent", err);
  }
});

async function refreshData() {
  try {
    const state: any = await invoke('get_app_state');
    products.value = state.products || {};
    isOffline.value = state.offline;

    // Re-select the current product to update its UI state if one is selected
    if (selectedProductName.value) {
      await selectProduct(selectedProductName.value);
    }
  } catch (err: any) {
    alert("Failed to fetch app state: " + err);
  }
}

async function selectProduct(name: string) {
  selectedProductName.value = name;
  selectedProductData.value = products.value[name];
  targetInstallVersion.value = selectedProductData.value.latest_version;
}

async function updateProduct() {
  isBusy.value = true;
  currentTaskName.value = 'Downloading & Applying Updates';
  progressData.value = { current: 0, total: 0, percent: 0, highest: 0 };
  logs.value.push(`--- Starting Update for ${selectedProductName.value} ---`);

  let success = false;

  while (!success) {
    try {
      await invoke('run_update', {
        productName: selectedProductName.value,
        targetVersion: targetInstallVersion.value || selectedProductData.value.latest_version,
        availableVersions: selectedProductData.value.versions
      });
      progressData.value.percent = 100;
      await refreshData();
      success = true;
    } catch (err: any) {
      const errorString = String(err);

      if (errorString.includes("currently running")) {
        if (confirm(`${selectedProductName.value} is currently running.\n\nDo you want to force close it to continue the update?`)) {
          logs.value.push('Attempting to force kill the process...');
          try {
            await invoke('force_kill_product', { productName: selectedProductName.value });
            logs.value.push('Process killed. Resuming update...');
            // Wait 1 second to give Windows time to release the file locks
            await new Promise(r => setTimeout(r, 1000));
            // Jump back to the top of the loop and try the update again!
          } catch (killErr) {
            logs.value.push(`Failed to kill process: ${killErr}`);
            alert(`Could not close the application automatically. Please close it manually.`);
            break;
          }
        } else {
          logs.value.push('Update cancelled by user (application running).');
          break; // User declined
        }
      } else if (errorString.includes("INSUFFICIENT_SPACE")) {
        // Handle disk space error
        const parts = errorString.split(":");
        const reqGb = (parseInt(parts[1]) / 1024 / 1024 / 1024).toFixed(2);
        const availGb = (parseInt(parts[2]) / 1024 / 1024 / 1024).toFixed(2);
        const msg = `Not enough disk space! You need at least ${reqGb} GB, but only have ${availGb} GB available.`;
        alert(msg);
        logs.value.push(`CRITICAL ERROR: ${msg}`);
        break;
      } else {
        logs.value.push(`ERROR: ${errorString}`);
        alert(`Update failed: ${errorString}`);
        break;
      }
    }
  }

  setTimeout(() => { isBusy.value = false; }, 1000);
}

async function launchApp() {
  isBusy.value = true;
  currentTaskName.value = 'Launching App';
  progressData.value = { current: 0, total: 0, percent: 100, highest: 0 }; // Fake full bar for launch
  try {
    // No longer passing serverUrl
    await invoke('launch_product', {
      productName: selectedProductName.value,
    });
  } catch (err: any) {
    alert(`Failed to launch: ${err}`);
  } finally {
    isBusy.value = false;
  }
}

async function repairInstallation() {
  if (!selectedProductData.value?.local_version) return;

  isBusy.value = true;
  currentTaskName.value = 'Repairing Installation';
  progressData.value = { current: 0, total: 0, percent: 0, highest: 0 }; // Reset progress
  logs.value.push(`--- Starting Repair Scan ---`);

  try {
    await invoke('repair_installation', {
      productName: selectedProductName.value,
      version: selectedProductData.value.local_version
    });

    logs.value.push(`Repair complete! The installation is fully valid.`);
    progressData.value.percent = 100;
  } catch (err: any) {
    logs.value.push(`ERROR: ${err}`);
    alert(err);
  } finally {
    setTimeout(() => { isBusy.value = false; }, 1000);
  }
}

async function uninstallProduct() {
  if (!confirm(`Are you sure you want to uninstall ${selectedProductName.value}?`)) return;

  isBusy.value = true;
  currentTaskName.value = 'Uninstalling Product';
  progressData.value = { current: 0, total: 0, percent: 100, highest: 0 };
  logs.value.push(`--- Uninstalling ${selectedProductName.value} ---`);

  let success = false;

  while (!success) {
    try {
      await invoke('uninstall_product', {
        productName: selectedProductName.value,
      });
      logs.value.push(`${selectedProductName.value} uninstalled.`);
      await refreshData();
      success = true;
    } catch (err: any) {
      const errorString = String(err);

      if (errorString.includes("currently running")) {
        if (confirm(`${selectedProductName.value} is currently running.\n\nDo you want to force close it to continue uninstalling?`)) {
          logs.value.push('Attempting to force kill the process...');
          try {
            await invoke('force_kill_product', { productName: selectedProductName.value });
            logs.value.push('Process killed. Resuming uninstall...');
            await new Promise(r => setTimeout(r, 1000));
            // loopback, try the uninstall again
          } catch (killErr) {
            alert(`Could not close the application automatically. Please close it manually.`);
            break;
          }
        } else {
          logs.value.push('Uninstall cancelled by user (application running).');
          break;
        }
      } else {
        alert(`Failed to uninstall: ${errorString}`);
        break;
      }
    }
  }

  isBusy.value = false;
}
</script>

<template>
  <div class="flex flex-col h-screen w-screen bg-background text-foreground font-sans overflow-hidden">

    <div v-if="isOffline" class="bg-destructive text-destructive-foreground text-center py-2 text-sm font-bold w-full z-50 shadow-md">
      Offline Mode: Server unreachable. You can only launch or uninstall existing applications.
    </div>

    <div class="flex flex-1 overflow-hidden">
      <aside class="w-64 bg-card p-4 flex flex-col border-r border-border">
        <h1 class="text-xl font-bold mb-6 text-primary">Launcher</h1>

        <div v-if="Object.keys(products).length === 0" class="text-muted-foreground text-sm">
          No products found.
        </div>

        <div class="flex flex-col gap-2 flex-1 overflow-y-auto">
          <Button
              v-for="(entry, name) in products"
              :key="name"
              :variant="selectedProductName === name ? 'default' : 'ghost'"
              class="justify-start w-full"
              @click="selectProduct(String(name))"
          >
            {{ name }}
          </Button>
        </div>

        <div class="mt-auto flex flex-col gap-2 pt-4 border-t border-border">
          <Button variant="secondary" @click="showLogsModal = true">
            View Logs
          </Button>
          <Button variant="outline" @click="refreshData">
            Refresh Data
          </Button>
        </div>
      </aside>

      <main class="flex-1 p-8 overflow-y-auto bg-muted/20">
        <Card v-if="selectedProductName" class="max-w-3xl mx-auto shadow-lg">
          <CardHeader>
            <CardTitle class="text-3xl">{{ selectedProductName }}</CardTitle>
            <CardDescription>Manage your installation and updates.</CardDescription>
          </CardHeader>

          <CardContent>
            <div class="flex gap-8 mb-6 text-sm">
              <div class="flex flex-col">
                <span class="text-muted-foreground">Local Version</span>
                <span class="font-mono font-medium text-lg">{{ selectedProductData.local_version || 'Not Installed' }}</span>
              </div>
              <div class="flex flex-col">
                <span class="text-muted-foreground">Latest Version</span>
                <span class="font-mono font-medium text-lg text-primary">{{ selectedProductData.latest_version }}</span>
              </div>
            </div>

            <div class="space-y-4 pt-6 border-t border-border">

              <div v-if="!selectedProductData.local_version" class="flex items-center gap-4">
                <Select v-model="targetInstallVersion" :disabled="isBusy || isOffline">
                  <SelectTrigger class="w-[180px]">
                    <SelectValue placeholder="Select version" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectGroup>
                      <SelectItem v-for="v in selectedProductData.versions" :key="v" :value="v">
                        Install v{{ v }}
                      </SelectItem>
                    </SelectGroup>
                  </SelectContent>
                </Select>

                <Button @click="updateProduct" :disabled="isBusy || isOffline" size="lg">
                  Install Product
                </Button>
              </div>

              <div v-if="selectedProductData.local_version" class="space-y-6">

                <div v-if="selectedProductData.local_version !== selectedProductData.latest_version" class="bg-blue-900/20 border border-blue-800 p-4 rounded-lg flex items-center justify-between">
                  <div>
                    <h4 class="font-bold text-blue-400">Update Available!</h4>
                    <p class="text-sm text-blue-200">Version {{ selectedProductData.latest_version }} is ready to install.</p>
                  </div>
                  <Button @click="updateProduct" :disabled="isBusy || isOffline" class="bg-blue-600 hover:bg-blue-500 text-white">
                    Update Now
                  </Button>
                </div>

                <div v-else class="text-green-500 font-bold flex items-center gap-2">
                  <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/><polyline points="22 4 12 14.01 9 11.01"/></svg>
                  Product is fully up to date!
                </div>

                <div class="flex gap-4 w-full">
                  <Button @click="launchApp" :disabled="isBusy" size="lg" class="flex-1 text-lg h-12">
                    Launch Product
                  </Button>
                  <Button variant="secondary" @click="repairInstallation" :disabled="isBusy || isOffline" size="lg" class="h-12">
                    Repair
                  </Button>
                  <Button variant="destructive" @click="uninstallProduct" :disabled="isBusy" size="lg" class="h-12">
                    Uninstall
                  </Button>
                </div>
              </div>

              <div v-if="isBusy" class="pt-6 space-y-2 animate-in fade-in slide-in-from-bottom-2">
                <div class="flex justify-between items-end">
                  <span class="text-sm font-medium text-muted-foreground">
                    {{ currentTaskName }}...
                  </span>
                  <button @click="showLogsModal = true" class="text-xs text-primary hover:underline">View Logs</button>
                </div>
                <Progress :model-value="progressData.percent" class="h-2 w-full" :class="{'animate-pulse': progressData.percent === 0}" />
              </div>

            </div>
          </CardContent>
        </Card>

        <div v-else class="flex h-full items-center justify-center text-muted-foreground">
          Select a product from the menu to manage it.
        </div>
      </main>
    </div>

    <Dialog :open="showLogsModal" @update:open="showLogsModal = $event">
      <DialogContent class="max-w-3xl h-[70vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>Process Logs</DialogTitle>
          <DialogDescription>Live output from the updater engine.</DialogDescription>
        </DialogHeader>

        <ScrollArea class="flex-1 w-full rounded-md border p-4 bg-black/90">
          <div class="font-mono text-sm text-green-400 space-y-1">
            <div v-for="(log, idx) in logs" :key="idx">{{ log }}</div>
            <div v-if="logs.length === 0" class="text-muted-foreground">Waiting for process...</div>
          </div>
        </ScrollArea>

        <DialogFooter>
          <Button variant="outline" @click="showLogsModal = false">Close</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>

  </div>
</template>