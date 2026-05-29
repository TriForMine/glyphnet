import { useMemo, useState } from "react";
import { SvgXml } from "react-native-svg";

import { scannerAdapter } from "@/adapters/scanner";
import { Pressable, Text, TextInput, View } from "@/tw";

export function EncodePanel() {
  const [payload, setPayload] = useState("hello glyphnet");
  const [svgPreview, setSvgPreview] = useState("");
  const [encodeError, setEncodeError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const payloadBytes = useMemo(() => new TextEncoder().encode(payload).length, [payload]);

  const runEncode = async () => {
    setLoading(true);
    try {
      setEncodeError(null);
      const svg = await scannerAdapter.encodeSvg(payload);
      if (svg.trim().startsWith("<svg")) {
        setSvgPreview(svg);
      } else {
        try {
          const parsed = JSON.parse(svg) as { error?: string };
          setSvgPreview("");
          setEncodeError(parsed.error ?? "encode_failed");
        } catch {
          setSvgPreview("");
          setEncodeError("encode_failed");
        }
      }
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

      <View className="rounded-3xl bg-white p-4 dark:bg-black">
        <Text className="text-xs font-semibold uppercase tracking-wide text-slate-500 dark:text-slate-400">
          SVG output
        </Text>
        {svgPreview ? (
          <View className="mt-2 overflow-hidden rounded-xl border border-slate-200 bg-white p-2 dark:border-slate-700">
            <SvgXml xml={svgPreview} width="100%" height={220} />
          </View>
        ) : (
          <Text
            selectable
            className="mt-2 font-mono text-xs leading-5 text-slate-700 dark:text-slate-100"
          >
            {encodeError ?? "No output yet"}
          </Text>
        )}
      </View>
    </View>
  );
}
