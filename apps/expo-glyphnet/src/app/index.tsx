import { ScanPanel } from "../features/scan/ScanPanel";
import { View } from "../tw";
import { SafeAreaView } from "react-native-safe-area-context";

export default function ScanScreen() {
  return (
    <SafeAreaView style={{ flex: 1 }} edges={["left", "right", "bottom"]}>
      <View className="flex-1 bg-slate-950 dark:bg-black">
        <ScanPanel />
      </View>
    </SafeAreaView>
  );
}
