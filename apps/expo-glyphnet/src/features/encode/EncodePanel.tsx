import { useState } from "react";
import { Button, StyleSheet, TextInput } from "react-native";

import { scannerAdapter } from "@/adapters/scanner";
import { ThemedText } from "@/components/themed-text";
import { ThemedView } from "@/components/themed-view";

export function EncodePanel() {
  const [payload, setPayload] = useState("hello glyphnet");
  const [svgPreview, setSvgPreview] = useState("");
  const [loading, setLoading] = useState(false);

  const runEncode = async () => {
    setLoading(true);
    try {
      const svg = await scannerAdapter.encodeSvg(payload);
      setSvgPreview(svg.slice(0, 140) + (svg.length > 140 ? "..." : ""));
    } finally {
      setLoading(false);
    }
  };

  return (
    <ThemedView type="backgroundElement" style={styles.card}>
      <ThemedText type="subtitle">Encode</ThemedText>
      <TextInput
        value={payload}
        onChangeText={setPayload}
        style={styles.input}
        placeholder="payload"
      />
      <Button title={loading ? "Encoding..." : "Mock Encode SVG"} disabled={loading} onPress={runEncode} />
      <ThemedText type="small" style={styles.output}>
        {svgPreview || "No output yet"}
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
