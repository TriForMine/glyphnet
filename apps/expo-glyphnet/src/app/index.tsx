import { ScanPanel } from "../features/scan/ScanPanel";
import { Text, View } from "../tw";
import { ScrollView } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

export default function ScanScreen() {
  return (
    <SafeAreaView style={{ flex: 1 }} edges={["top"]}>
      <View className="flex-1 bg-slate-50 dark:bg-neutral-950">
        <View className="px-5 pt-4 pb-3">
          <View className="rounded-2xl bg-white/90 px-4 py-3 dark:bg-neutral-900">
            <Text className="text-[12px] font-semibold uppercase tracking-[2px] text-sky-600 dark:text-sky-400">
              GlyphNet Mobile
            </Text>
            <Text className="mt-1 text-2xl font-semibold text-slate-900 dark:text-neutral-100">
              Scanner
            </Text>
            <Text className="mt-1 text-sm text-slate-500 dark:text-neutral-400">
              Frame the symbol, scan quickly, and validate authenticity.
            </Text>
          </View>
        </View>

        <ScrollView
          style={{ flex: 1 }}
          contentInsetAdjustmentBehavior="automatic"
          contentContainerStyle={{ paddingHorizontal: 20, paddingBottom: 24 }}
          showsVerticalScrollIndicator={false}
        >
          <ScanPanel />
        </ScrollView>
      </View>
    </SafeAreaView>
  );
}
