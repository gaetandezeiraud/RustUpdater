<script setup lang="ts">
import { computed } from 'vue';
import { useDownloadQueue } from '@/composables/useDownloadQueue';
import { Button } from '@/components/ui/button';
import { Progress } from '@/components/ui/progress';

const { queue, activeItem, progress, cancelCurrent, removeFromQueue } = useDownloadQueue();

const hasActivity = computed(() => activeItem.value !== null || queue.value.length > 0);

const taskLabel: Record<string, string> = {
  install: 'Installing',
  update: 'Updating',
  repair: 'Repairing',
};
</script>

<template>
  <div v-if="hasActivity" class="space-y-2">
    <p class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">Download Manager</p>

    <!-- Active download -->
    <div v-if="activeItem" class="bg-muted/40 rounded-md p-2.5 space-y-2">
      <div class="flex items-center justify-between gap-2">
        <div class="min-w-0 flex-1">
          <p class="text-xs font-semibold truncate">{{ activeItem.productName }}</p>
          <p class="text-xs text-muted-foreground leading-none mt-0.5">
            {{ taskLabel[activeItem.type] ?? activeItem.type }}…
          </p>
        </div>
        <!-- Cancel is only allowed for fresh installs -->
        <Button
          v-if="activeItem.type === 'install'"
          variant="destructive"
          size="sm"
          class="h-6 px-2 text-xs shrink-0"
          @click="cancelCurrent"
        >
          Stop
        </Button>
      </div>
      <Progress
        :model-value="progress.percent"
        class="h-1.5"
        :class="{ 'animate-pulse': progress.percent === 0 }"
      />
      <p class="text-xs text-muted-foreground text-right leading-none">
        {{ progress.percent.toFixed(0) }}%
      </p>
    </div>

    <!-- Queued items -->
    <div v-if="queue.length > 0" class="space-y-1">
      <p class="text-xs text-muted-foreground">Up next:</p>
      <div
        v-for="item in queue"
        :key="item.productName"
        class="flex items-center justify-between bg-muted/20 rounded px-2 py-1 gap-1"
      >
        <div class="min-w-0 flex-1">
          <p class="text-xs truncate font-medium">{{ item.productName }}</p>
          <p class="text-xs text-muted-foreground capitalize leading-none">{{ item.type }}</p>
        </div>
        <Button
          variant="ghost"
          size="sm"
          class="h-5 w-5 p-0 text-muted-foreground hover:text-destructive shrink-0"
          @click="removeFromQueue(item.productName)"
        >
          ✕
        </Button>
      </div>
    </div>
  </div>
</template>

