import { CameraView, useCameraPermissions } from "expo-camera";
import { Directory, File, Paths } from "expo-file-system";
import * as NavigationBar from "expo-navigation-bar";
import * as ImageManipulator from "expo-image-manipulator";
import { useMemo, useRef, useState } from "react";
import { Modal, Vibration } from "react-native";
import { Platform, useColorScheme } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { useEffect } from "react";

import { scannerAdapter } from "@/adapters/scanner";
import { Image } from "@/tw/image";
import { Pressable, Text, TextInput, View } from "@/tw";

const MODES = ["print", "screen", "burst"] as const;
// Wide ribbon-like scan guide (GlyphNet print profile), centered in portrait view.
const GUIDE_ROI = { x: 0.06, y: 0.34, w: 0.88, h: 0.26 } as const;

export function ScanPanel() {
    const cameraRef = useRef<any>(null);
    const insets = useSafeAreaInsets();
    const colorScheme = useColorScheme();
    const [permission, requestPermission] = useCameraPermissions();
    const [mode, setMode] = useState<(typeof MODES)[number]>("print");
    const [verifyKeyHex, setVerifyKeyHex] = useState("");
    const [result, setResult] = useState("");
    const [resultOpen, setResultOpen] = useState(false);
    const [debugEnabled, setDebugEnabled] = useState(false);
    const [lastCaptureDataUri, setLastCaptureDataUri] = useState<string | null>(
        null,
    );
    const [lastCaptureUri, setLastCaptureUri] = useState<string | null>(null);
    const [lastCaptureWidth, setLastCaptureWidth] = useState<number | null>(
        null,
    );
    const [lastCaptureHeight, setLastCaptureHeight] = useState<number | null>(
        null,
    );
    const [captureMs, setCaptureMs] = useState<number | null>(null);
    const [scanMs, setScanMs] = useState<number | null>(null);
    const [torchEnabled, setTorchEnabled] = useState(false);
    const [loading, setLoading] = useState(false);
    const [debugSaveMessage, setDebugSaveMessage] = useState<string | null>(
        null,
    );

    const statusLabel = useMemo(() => {
        if (!permission) {
            return "Checking camera permission...";
        }
        return permission.granted
            ? "Camera ready"
            : "Camera access required for live scanning";
    }, [permission]);

    useEffect(() => {
        if (Platform.OS !== "android" || colorScheme !== "dark") {
            return;
        }
        const applyNavStyle = async () => {
            try {
                await NavigationBar.setBackgroundColorAsync("#020617");
                await NavigationBar.setButtonStyleAsync("light");
            } catch {
                // Ignore runtime/platform cases where nav bar styling is unavailable.
            }
        };
        void applyNavStyle();
    }, [colorScheme]);

    const runScan = async () => {
        setLoading(true);
        try {
            if (!cameraRef.current?.takePictureAsync) {
                setResult("Camera capture is not ready yet.");
                return;
            }
            const captureStart = Date.now();
            const shot = await cameraRef.current.takePictureAsync({
                base64: true,
                quality: 0.9,
                shutterSound: false,
            });
            const captureElapsed = Date.now() - captureStart;
            setCaptureMs(captureElapsed);
            if (!shot?.base64) {
                setResult("Failed to capture image from camera.");
                return;
            }
            setLastCaptureUri(shot.uri ?? null);
            setLastCaptureWidth(
                typeof shot.width === "number" ? shot.width : null,
            );
            setLastCaptureHeight(
                typeof shot.height === "number" ? shot.height : null,
            );
            if (debugEnabled) {
                setLastCaptureDataUri(`data:image/jpeg;base64,${shot.base64}`);
            }
            const scanStart = Date.now();
            const json = await scannerAdapter.scanStill({
                mode,
                verifyKeyHex: verifyKeyHex || undefined,
                verifyKeyId: 1,
                imageBase64: shot.base64,
                roiX: GUIDE_ROI.x,
                roiY: GUIDE_ROI.y,
                roiW: GUIDE_ROI.w,
                roiH: GUIDE_ROI.h,
            });
            const scanElapsed = Date.now() - scanStart;
            setScanMs(scanElapsed);
            setResult(JSON.stringify(json, null, 2));
            if (json.ok) {
                Vibration.vibrate(18);
            } else {
                Vibration.vibrate([0, 24, 36, 24]);
            }
            setResultOpen(true);
        } finally {
            setLoading(false);
        }
    };

    const parsedResult = useMemo(() => {
        try {
            return JSON.parse(result) as {
                ok?: boolean;
                payload_utf8_lossy?: string;
                payload_len?: number;
                error?: string;
                mode?: string;
            };
        } catch {
            return null;
        }
    }, [result]);

    const saveDebugBundle = async () => {
        setDebugSaveMessage(null);
        if (!lastCaptureUri || !lastCaptureWidth || !lastCaptureHeight) {
            setDebugSaveMessage("No captured image available yet.");
            return;
        }
        try {
            const stamp = Date.now();
            const outDir = new Directory(Paths.document, `glyphnet-debug-${stamp}`);
            outDir.create({ idempotent: true, intermediates: true });

            const captureOut = new File(outDir, "capture.jpg");
            captureOut.create({ overwrite: true, intermediates: true });
            new File(lastCaptureUri).copy(captureOut, { overwrite: true });

            const crop = {
                originX: Math.max(
                    0,
                    Math.floor(lastCaptureWidth * GUIDE_ROI.x),
                ),
                originY: Math.max(
                    0,
                    Math.floor(lastCaptureHeight * GUIDE_ROI.y),
                ),
                width: Math.max(1, Math.floor(lastCaptureWidth * GUIDE_ROI.w)),
                height: Math.max(
                    1,
                    Math.floor(lastCaptureHeight * GUIDE_ROI.h),
                ),
            };
            const cropped = await ImageManipulator.manipulateAsync(
                lastCaptureUri,
                [{ crop }],
                { compress: 1, format: ImageManipulator.SaveFormat.JPEG },
            );
            const roiOut = new File(outDir, "roi.jpg");
            roiOut.create({ overwrite: true, intermediates: true });
            new File(cropped.uri).copy(roiOut, { overwrite: true });

            const debugJson = {
                ts: stamp,
                mode,
                roi: GUIDE_ROI,
                capture: {
                    width: lastCaptureWidth,
                    height: lastCaptureHeight,
                    captureMs,
                    scanMs,
                },
                result: parsedResult ?? result,
            };
            const jsonOut = new File(outDir, "result.json");
            jsonOut.create({ overwrite: true, intermediates: true });
            jsonOut.write(JSON.stringify(debugJson, null, 2), {
                encoding: "utf8",
            });

            try {
                const Sharing = await import("expo-sharing");
                if (await Sharing.isAvailableAsync()) {
                    await Sharing.shareAsync(roiOut.uri, {
                        mimeType: "image/jpeg",
                        dialogTitle: "Share GlyphNet ROI debug image",
                    });
                }
            } catch {
                // no-op: sharing may be unavailable in current runtime.
            }

            setDebugSaveMessage(`Saved debug bundle: ${outDir.uri}`);
        } catch (error) {
            setDebugSaveMessage(
                error instanceof Error
                    ? `Debug save failed: ${error.message}`
                    : "Debug save failed.",
            );
        }
    };

    return (
        <View className="flex-1">
            {permission?.granted ? (
                <View className="relative h-full w-full">
                    <CameraView
                        ref={cameraRef}
                        style={{ width: "100%", height: "100%" }}
                        facing="back"
                        animateShutter={false}
                        enableTorch={torchEnabled}
                    />
                    <View
                        pointerEvents="none"
                        style={{
                            position: "absolute",
                            inset: 0,
                            backgroundColor: "rgba(2, 6, 23, 0.18)",
                        }}
                    />
                    <View
                        pointerEvents="none"
                        className="absolute left-4 right-4"
                        style={{ top: Math.max(insets.top, 8) }}
                    >
                        <Text className="text-center text-lg font-semibold tracking-wide text-white">
                            GlyphNet
                        </Text>
                    </View>
                    <View
                        pointerEvents="none"
                        style={{
                            position: "absolute",
                            left: `${GUIDE_ROI.x * 100}%`,
                            top: `${GUIDE_ROI.y * 100}%`,
                            width: `${GUIDE_ROI.w * 100}%`,
                            height: `${GUIDE_ROI.h * 100}%`,
                            borderWidth: 2,
                            borderColor: "#38bdf8",
                            borderRadius: 12,
                        }}
                    />
                    <View className="absolute bottom-0 left-0 right-0 p-4">
                        <View className="rounded-2xl bg-black/55 p-3">
                            <Text className="text-center text-xs font-medium text-slate-100">
                                Align the GlyphNet code inside the blue ribbon
                                frame
                            </Text>
                            <View className="mt-3 flex-row gap-2">
                                {MODES.map((candidate) => (
                                    <Pressable
                                        key={candidate}
                                        onPress={() => setMode(candidate)}
                                        className={`flex-1 rounded-xl border px-3 py-2 ${
                                            mode === candidate
                                                ? "border-sky-400 bg-sky-500/30"
                                                : "border-slate-400/60 bg-slate-900/40"
                                        }`}
                                    >
                                        <Text className="text-center text-xs font-semibold uppercase tracking-wide text-slate-100">
                                            {candidate}
                                        </Text>
                                    </Pressable>
                                ))}
                            </View>
                            <View className="mt-3 flex-row gap-2">
                                <Pressable
                                    onPress={() => setTorchEnabled((v) => !v)}
                                    className={`flex-1 items-center rounded-xl border px-3 py-2 ${
                                        torchEnabled
                                            ? "border-amber-300 bg-amber-500/30"
                                            : "border-slate-400/60 bg-slate-900/40"
                                    }`}
                                >
                                    <Text className="text-xs font-semibold uppercase tracking-wide text-slate-100">
                                        Torch: {torchEnabled ? "On" : "Off"}
                                    </Text>
                                </Pressable>
                            </View>
                            <Pressable
                                onPress={() => setDebugEnabled((v) => !v)}
                                className={`mt-3 items-center rounded-xl border px-3 py-2 ${
                                    debugEnabled
                                        ? "border-amber-300 bg-amber-500/25"
                                        : "border-slate-400/60 bg-slate-900/40"
                                }`}
                            >
                                <Text className="text-xs font-semibold uppercase tracking-wide text-slate-100">
                                    Debug: {debugEnabled ? "On" : "Off"}
                                </Text>
                            </Pressable>
                            <TextInput
                                value={verifyKeyHex}
                                onChangeText={setVerifyKeyHex}
                                autoCapitalize="none"
                                placeholder="verification key (optional)"
                                placeholderTextColor="#94a3b8"
                                className="mt-3 rounded-xl border border-slate-500/60 bg-slate-900/60 px-3 py-3 text-slate-100"
                            />
                            <Pressable
                                onPress={runScan}
                                disabled={loading}
                                className="mt-3 items-center rounded-xl bg-sky-600 px-4 py-3 active:opacity-80 disabled:opacity-60"
                            >
                                <Text className="text-base font-semibold text-white">
                                    {loading ? "Scanning..." : "Scan Still"}
                                </Text>
                            </Pressable>
                        </View>
                    </View>
                </View>
            ) : (
                <View className="flex-1 items-center justify-center px-6">
                    <Text className="text-center text-sm text-slate-600 dark:text-neutral-300">
                        {statusLabel}
                    </Text>
                    <Pressable
                        onPress={requestPermission}
                        className="mt-4 rounded-xl bg-sky-600 px-4 py-2 active:opacity-80"
                    >
                        <Text className="text-sm font-semibold text-white">
                            Allow Camera
                        </Text>
                    </Pressable>
                </View>
            )}

            <Modal
                visible={resultOpen}
                transparent
                animationType="fade"
                onRequestClose={() => setResultOpen(false)}
            >
                <View className="flex-1 items-center justify-center bg-black/65 p-5">
                    <View className="w-full max-w-[560px] rounded-2xl bg-slate-900 p-4">
                        <Text className="text-xs font-semibold uppercase tracking-wide text-slate-400">
                            Scan Result
                        </Text>
                        {parsedResult ? (
                            <View className="mt-2 gap-3">
                                <View
                                    className={`rounded-xl px-3 py-2 ${
                                        parsedResult.ok
                                            ? "bg-emerald-500/20"
                                            : "bg-rose-500/20"
                                    }`}
                                >
                                    <Text
                                        className={`text-sm font-semibold ${
                                            parsedResult.ok
                                                ? "text-emerald-300"
                                                : "text-rose-300"
                                        }`}
                                    >
                                        {parsedResult.ok
                                            ? "Scan succeeded"
                                            : "Scan failed"}
                                    </Text>
                                    {!!parsedResult.mode && (
                                        <Text className="mt-1 text-xs text-slate-300">
                                            Mode: {parsedResult.mode}
                                        </Text>
                                    )}
                                    <Text className="mt-1 text-xs text-slate-300">
                                        Capture: {captureMs ?? "-"} ms | Scan:{" "}
                                        {scanMs ?? "-"} ms
                                    </Text>
                                </View>

                                {parsedResult.ok ? (
                                    <View className="rounded-xl bg-slate-800 p-3">
                                        <Text className="text-xs uppercase tracking-wide text-slate-400">
                                            Payload
                                        </Text>
                                        <Text
                                            selectable
                                            className="mt-1 text-sm text-slate-100"
                                        >
                                            {parsedResult.payload_utf8_lossy ||
                                                "(empty)"}
                                        </Text>
                                        <Text className="mt-1 text-xs text-slate-400">
                                            {parsedResult.payload_len ?? 0}{" "}
                                            bytes
                                        </Text>
                                    </View>
                                ) : (
                                    <View className="rounded-xl bg-slate-800 p-3">
                                        <Text className="text-xs uppercase tracking-wide text-slate-400">
                                            Error
                                        </Text>
                                        <Text
                                            selectable
                                            className="mt-1 text-sm text-rose-300"
                                        >
                                            {parsedResult.error ||
                                                "Unknown error"}
                                        </Text>
                                    </View>
                                )}

                                {debugEnabled && lastCaptureDataUri ? (
                                    <View className="rounded-xl bg-slate-800 p-3">
                                        <Text className="text-xs uppercase tracking-wide text-slate-400">
                                            Debug Capture + ROI
                                        </Text>
                                        <View className="relative mt-2 overflow-hidden rounded-lg border border-slate-600">
                                            <Image
                                                source={{
                                                    uri: lastCaptureDataUri,
                                                }}
                                                contentFit="fill"
                                                style={{
                                                    width: "100%",
                                                    height: 180,
                                                    backgroundColor: "#000",
                                                }}
                                            />
                                            <View
                                                pointerEvents="none"
                                                style={{
                                                    position: "absolute",
                                                    left: `${GUIDE_ROI.x * 100}%`,
                                                    top: `${GUIDE_ROI.y * 100}%`,
                                                    width: `${GUIDE_ROI.w * 100}%`,
                                                    height: `${GUIDE_ROI.h * 100}%`,
                                                    borderWidth: 2,
                                                    borderColor: "#22d3ee",
                                                    borderRadius: 8,
                                                }}
                                            />
                                        </View>
                                        <Pressable
                                            onPress={() => {
                                                void saveDebugBundle();
                                            }}
                                            className="mt-3 items-center rounded-xl border border-cyan-400/50 bg-cyan-500/20 px-3 py-2"
                                        >
                                            <Text className="text-xs font-semibold uppercase tracking-wide text-cyan-100">
                                                Save Debug Bundle
                                            </Text>
                                        </Pressable>
                                        {debugSaveMessage ? (
                                            <Text className="mt-2 text-xs text-slate-300">
                                                {debugSaveMessage}
                                            </Text>
                                        ) : null}
                                    </View>
                                ) : null}

                                <Text
                                    selectable
                                    className="font-mono text-[11px] leading-5 text-slate-400"
                                >
                                    {result}
                                </Text>
                            </View>
                        ) : (
                            <Text
                                selectable
                                className="mt-2 font-mono text-xs leading-5 text-slate-100"
                            >
                                {result}
                            </Text>
                        )}

                        <View className="mt-4 flex-row gap-2">
                            <Pressable
                                onPress={() => setResultOpen(false)}
                                className="flex-1 items-center rounded-xl border border-slate-500 px-4 py-3"
                            >
                                <Text className="text-sm font-semibold text-slate-100">
                                    Close
                                </Text>
                            </Pressable>
                            <Pressable
                                onPress={() => {
                                    setResultOpen(false);
                                    void runScan();
                                }}
                                className="flex-1 items-center rounded-xl bg-sky-600 px-4 py-3"
                            >
                                <Text className="text-sm font-semibold text-white">
                                    Scan Again
                                </Text>
                            </Pressable>
                        </View>
                    </View>
                </View>
            </Modal>
        </View>
    );
}
