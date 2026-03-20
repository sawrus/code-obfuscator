import type { Plugin } from "@opencode-ai/plugin"

export const TelegramNotificationPlugin: Plugin = async ({ $, client, directory }) => {
  return {
    event: async ({ event }) => {
      if (event.type === "session.idle") {
        const botToken = process.env.OPENCODE_TELEGRAM_BOT_TOKEN
        const chatId = process.env.OPENCODE_TELEGRAM_CHAT_ID
        
        if (!botToken || !chatId) return
        
        const sessionID = event.properties.sessionID
        let text = "✅ Задача завершена"
        
        try {
          const sessionResult = await client.session.get({ path: { id: sessionID } })
          const session = sessionResult.data
          
          if (!sessionResult.error && session) {
            text = `✅ ${session.title || "Задача завершена"}`
            
            if (session.summary) {
              text += `\n📊 +${session.summary.additions} -${session.summary.deletions} в ${session.summary.files} файл(ах)`
            }
            
            const messagesResult = await client.session.messages({ path: { id: sessionID } })
            if (!messagesResult.error && messagesResult.data?.length) {
              const lastMessage = messagesResult.data[messagesResult.data.length - 1]
              const textParts = lastMessage.parts?.filter(p => p.type === "text") || []
              if (textParts.length) {
                const lastText = textParts.map(p => (p as any).text).join("").slice(0, 500)
                text += `\n\n${lastText}`
              }
            }
          }
        } catch (e) {
          await $`echo "Error: ${e}" >> ${directory}/.opencode/telegram-debug.log`
        }

        try {
          await fetch(
            `https://api.telegram.org/bot${botToken}/sendMessage`,
            {
              method: "POST",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify({
                chat_id: chatId,
                text
              })
            }
          )
        } catch {}
      }
    },
  }
}
