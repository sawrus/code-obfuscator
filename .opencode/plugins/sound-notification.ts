import type { Plugin } from "@opencode-ai/plugin"

export const SoundNotificationPlugin: Plugin = async ({ $ }) => {
  return {
    event: async ({ event }) => {
      if (event.type === "session.idle") {
        try {
          await $`afplay /System/Library/Sounds/Glass.aiff`
        } catch {}
      }
    },
  }
}
