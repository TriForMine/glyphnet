import { useState } from "react";
import { Button, StyleSheet, TextInput, View } from "react-native";

import { scannerAdapter } from "@/adapters/scanner";
import { ThemedText } from "@/components/themed-text";
import { ThemedView } from "@/components/themed-view";

export function ScanPanel() {
  const [mode, setMode] = useState("print");
  const [verifyKeyHex, setVerifyKeyHex] = useState("");
  const [result, setResult] = useState("No scan yet");
  const [loading, setLoading] = useState(false);

  const runScan = async () => {
    setLoading(true);
    try {
      const json = await scannerAdapter.scanStill({
        mode: mode as "print" | "screen" | "burst",
        verifyKeyHex: verifyKeyHex || undefined,
        verifyKeyId: 1,
      });
      setResult(JSON.stringify(json, null, 2));
    } finally {
      setLoading(false);
    }
  };

  return (
    <ThemedView type="backgroundElement" style={styles.card}>
      <ThemedText type="subtitle">Scan</ThemedText>
      <TextInput
        value={mode}
        onChangeText={setMode}
        autoCapitalize="none"
        style={styles.input}
        placeholder="mode: print|screen|burst"
      />
      <TextInput
        value={verifyKeyHex}
        onChangeText={setVerifyKeyHex}
        autoCapitalize="none"
        style={styles.input}
        placeholder="optional verify key hex"
      />
      <Button title={loading ? "Scanning..." : "Mock Scan"} disabled={loading} onPress={runScan} />
      <ThemedText type="small" style={styles.output}>
        {result}
      </ThemedText>
    </ThemedView>
  );
}

const styles = StyleSheet.create({
  card: {
    width: "100%",
    gap: 8,
    borderRadius: 14,
    padding: 12,
  },
  input: {
    borderWidth: 1,
    borderColor: "#c8d4e7",
    borderRadius: 8,
    paddingHorizontal: 10,
    paddingVertical: 8,
    color: "#112035",
    backgroundColor: "#ffffff",
  },
  output: {
    marginTop: 4,
    fontFamily: "monospace",
  },
});
