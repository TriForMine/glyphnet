import { useMemo, useState } from "react";

import { scannerAdapter } from "@/adapters/scanner";
import { Pressable, Text, TextInput, View } from "@/tw";

export function EncodePanel() {
  const [payload, setPayload] = useState("hello glyphnet");
  const [svgPreview, setSvgPreview] = useState("");
  const [loading, setLoading] = useState(false);

  const payloadBytes = useMemo(() => new TextEncoder().encode(payload).length, [payload]);

  const runEncode = async () => {
    setLoading(true);
    try {
      const svg = await scannerAdapter.encodeSvg(payload);
      setSvgPreview(svg);
    } finally {
      setLoading(false);
    }
  };

  return (
    <View className="gap-3">
      <View className="rounded-3xl bg-white p-4 dark:bg-neutral-900">
        <Text className="text-sm font-medium text-slate-500 dark:text-neutral-400">
          Payload
        </Text>
        <TextInput
          value={payload}
          onChangeText={setPayload}
          multiline
          className="mt-3 min-h-[120px] rounded-2xl border border-slate-200 bg-slate-50 px-4 py-3 text-base text-slate-900 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
          placeholder="Enter payload text"
          placeholderTextColor="#64748b"
        />
        <View className="mt-3 flex-row items-center justify-between">
          <Text className="text-xs text-slate-500 dark:text-neutral-400">
            {payloadBytes} bytes
          </Text>
          <Pressable
            onPress={runEncode}
            disabled={loading}
            className="rounded-xl bg-violet-600 px-4 py-2 active:opacity-80 disabled:opacity-60"
          >
            <Text className="text-sm font-semibold text-white">
              {loading ? "Encoding..." : "Encode SVG"}
            </Text>
          </Pressable>
        </View>
      </View>

      <View className="rounded-3xl bg-slate-900 p-4 dark:bg-black">
        <Text className="text-xs font-semibold uppercase tracking-wide text-slate-400">
          SVG output
        </Text>
        <Text selectable className="mt-2 font-mono text-xs leading-5 text-slate-100">
          {svgPreview || "No output yet"}
        </Text>
      </View>
    </View>
  );
}

