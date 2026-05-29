import * as FileSystem from "expo-file-system";
import * as MediaLibrary from "expo-media-library";
import * as Print from "expo-print";
import { useMemo, useState } from "react";
import { SvgXml } from "react-native-svg";

import { scannerAdapter } from "@/adapters/scanner";
import { Pressable, Text, TextInput, View } from "@/tw";

function getWritableBaseDir(): string | null {
  const fsAny = FileSystem as unknown as {
    documentDirectory?: string | null;
    cacheDirectory?: string | null;
    Paths?: {
      document?: { uri?: string };
      cache?: { uri?: string };
    };
  };
  return (
    fsAny.documentDirectory ??
    fsAny.cacheDirectory ??
    fsAny.Paths?.document?.uri ??
    fsAny.Paths?.cache?.uri ??
    null
  );
}

export function EncodePanel() {
  const [payload, setPayload] = useState("hello glyphnet");
  const [svgPreview, setSvgPreview] = useState("");
  const [encodeError, setEncodeError] = useState<string | null>(null);
  const [actionMessage, setActionMessage] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const payloadBytes = useMemo(() => new TextEncoder().encode(payload).length, [payload]);

  const runEncode = async () => {
    setLoading(true);
    try {
      setEncodeError(null);
      setActionMessage(null);
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

  const saveSvg = async () => {
    if (!svgPreview) {
      setActionMessage("No SVG to save yet.");
      return;
    }
    try {
      const dir = getWritableBaseDir();
      if (!dir) {
        setActionMessage("Unable to access local storage.");
        return;
      }
      const uri = `${dir}glyphnet-${Date.now()}.svg`;
      await FileSystem.writeAsStringAsync(uri, svgPreview, {
        encoding: FileSystem.EncodingType.UTF8,
      });
      let savedToLibrary = false;
      try {
        const perm = await MediaLibrary.requestPermissionsAsync();
        if (perm.granted) {
          await MediaLibrary.saveToLibraryAsync(uri);
          savedToLibrary = true;
        }
      } catch {
        // Ignore and fall back to share/export.
      }

      // Load sharing lazily so Expo Go / unsupported runtimes never fail at import time.
      try {
        const Sharing = await import("expo-sharing");
        if (await Sharing.isAvailableAsync()) {
          await Sharing.shareAsync(uri, {
            mimeType: "image/svg+xml",
            dialogTitle: "Share GlyphNet SVG",
          });
          setActionMessage(
            savedToLibrary
              ? "SVG saved to library and share sheet opened."
              : "SVG exported and share sheet opened.",
          );
        } else {
          setActionMessage(
            savedToLibrary ? "SVG saved to library." : `SVG saved in app storage: ${uri}`,
          );
        }
      } catch {
        setActionMessage(
          savedToLibrary ? "SVG saved to library." : `SVG saved in app storage: ${uri}`,
        );
      }
    } catch (error) {
      setActionMessage(
        error instanceof Error ? `Save failed: ${error.message}` : "Save failed.",
      );
    }
  };

  const printSvg = async () => {
    if (!svgPreview) {
      setActionMessage("No SVG to print yet.");
      return;
    }
    try {
      await Print.printAsync({
        html: `<!doctype html><html><body style="margin:0;display:flex;align-items:center;justify-content:center;background:#fff;">${svgPreview}</body></html>`,
      });
      setActionMessage("Print dialog opened.");
    } catch (error) {
      setActionMessage(
        error instanceof Error ? `Print failed: ${error.message}` : "Print failed.",
      );
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
        <View className="mt-3 flex-row gap-2">
          <Pressable
            onPress={saveSvg}
            className="flex-1 items-center rounded-xl border border-slate-300 px-3 py-2 dark:border-slate-700"
          >
            <Text className="text-sm font-semibold text-slate-700 dark:text-slate-200">
              Save SVG
            </Text>
          </Pressable>
          <Pressable
            onPress={printSvg}
            className="flex-1 items-center rounded-xl border border-slate-300 px-3 py-2 dark:border-slate-700"
          >
            <Text className="text-sm font-semibold text-slate-700 dark:text-slate-200">
              Print
            </Text>
          </Pressable>
        </View>
        {actionMessage ? (
          <Text className="mt-2 text-xs text-slate-500 dark:text-slate-400">{actionMessage}</Text>
        ) : null}
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
