import { CameraView, useCameraPermissions } from "expo-camera";
import { useMemo, useRef, useState } from "react";

import { scannerAdapter } from "@/adapters/scanner";
import { Pressable, Text, TextInput, View } from "@/tw";

const MODES = ["print", "screen", "burst"] as const;

export function ScanPanel() {
  const cameraRef = useRef<any>(null);
  const [permission, requestPermission] = useCameraPermissions();
  const [mode, setMode] = useState<(typeof MODES)[number]>("print");
  const [verifyKeyHex, setVerifyKeyHex] = useState("");
  const [result, setResult] = useState("No scan yet");
  const [loading, setLoading] = useState(false);

  const statusLabel = useMemo(() => {
    if (!permission) {
      return "Checking camera permission...";
    }
    return permission.granted
      ? "Camera ready"
      : "Camera access required for live scanning";
  }, [permission]);

  const runScan = async () => {
    setLoading(true);
    try {
      if (!cameraRef.current?.takePictureAsync) {
        setResult("Camera capture is not ready yet.");
        return;
      }
      const shot = await cameraRef.current.takePictureAsync({
        base64: true,
        quality: 0.9,
      });
      if (!shot?.base64) {
        setResult("Failed to capture image from camera.");
        return;
      }
      const json = await scannerAdapter.scanStill({
        mode,
        verifyKeyHex: verifyKeyHex || undefined,
        verifyKeyId: 1,
        imageBase64: shot.base64,
      });
      setResult(JSON.stringify(json, null, 2));
    } finally {
      setLoading(false);
    }
  };

  return (
    <View className="gap-3">
      <View className="overflow-hidden rounded-3xl bg-white p-4 dark:bg-neutral-900">
        <Text className="text-sm font-medium text-slate-500 dark:text-neutral-400">
          Live Preview
        </Text>
        <View className="mt-3 h-56 overflow-hidden rounded-2xl bg-slate-200 dark:bg-neutral-800">
          {permission?.granted ? (
            <CameraView
              ref={cameraRef}
              style={{ width: "100%", height: "100%" }}
              facing="back"
            />
          ) : (
            <View className="flex-1 items-center justify-center px-6">
              <Text className="text-center text-sm text-slate-600 dark:text-neutral-300">
                {statusLabel}
              </Text>
              <Pressable
                onPress={requestPermission}
                className="mt-4 rounded-xl bg-sky-600 px-4 py-2 active:opacity-80"
              >
                <Text className="text-sm font-semibold text-white">Allow Camera</Text>
              </Pressable>
            </View>
          )}
        </View>
      </View>

      <View className="rounded-3xl bg-white p-4 dark:bg-neutral-900">
        <Text className="text-sm font-medium text-slate-500 dark:text-neutral-400">
          Mode
        </Text>
        <View className="mt-3 flex-row gap-2">
          {MODES.map((candidate) => (
            <Pressable
              key={candidate}
              onPress={() => setMode(candidate)}
              className={`flex-1 rounded-xl border px-3 py-2 ${
                mode === candidate
                  ? "border-sky-400 bg-sky-100 dark:border-sky-500 dark:bg-sky-900/40"
                  : "border-slate-200 bg-slate-100 dark:border-neutral-700 dark:bg-neutral-800"
              }`}
            >
              <Text className="text-center text-xs font-semibold uppercase tracking-wide text-slate-800 dark:text-neutral-100">
                {candidate}
              </Text>
            </Pressable>
          ))}
        </View>

        <Text className="mt-4 text-sm font-medium text-slate-500 dark:text-neutral-400">
          Verification key (optional)
        </Text>
        <TextInput
          value={verifyKeyHex}
          onChangeText={setVerifyKeyHex}
          autoCapitalize="none"
          placeholder="hex public key"
          placeholderTextColor="#64748b"
          className="mt-2 rounded-xl border border-slate-200 bg-slate-50 px-3 py-3 text-slate-900 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
        />

        <Pressable
          onPress={runScan}
          disabled={loading}
          className="mt-4 items-center rounded-xl bg-sky-600 px-4 py-3 active:opacity-80 disabled:opacity-60"
        >
          <Text className="text-base font-semibold text-white">
            {loading ? "Scanning..." : "Scan Still"}
          </Text>
        </Pressable>
      </View>

      <View className="rounded-3xl bg-slate-900 p-4 dark:bg-black">
        <Text className="text-xs font-semibold uppercase tracking-wide text-slate-400">
          Result
        </Text>
        <Text selectable className="mt-2 font-mono text-xs leading-5 text-slate-100">
          {result}
        </Text>
      </View>
    </View>
  );
}
