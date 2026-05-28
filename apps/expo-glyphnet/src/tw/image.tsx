import { Image as RNImage } from "expo-image";
import React from "react";
import { StyleSheet } from "react-native";
import { useCssElement } from "react-native-css";
import Animated from "react-native-reanimated";

const AnimatedExpoImage = Animated.createAnimatedComponent(RNImage);

function CSSImage(props: React.ComponentProps<typeof AnimatedExpoImage>) {
  const { objectFit, objectPosition, ...style } =
    (StyleSheet.flatten(props.style) as
      | ({ objectFit?: unknown; objectPosition?: unknown } & object)
      | undefined) || {};

  return (
    <AnimatedExpoImage
      contentFit={objectFit as never}
      contentPosition={objectPosition as never}
      {...props}
      source={
        typeof props.source === "string" ? { uri: props.source } : props.source
      }
      style={style}
    />
  );
}

export type ImageProps = React.ComponentProps<typeof CSSImage> & {
  className?: string;
};

export const Image = (props: ImageProps) => {
  return useCssElement(CSSImage, props, { className: "style" });
};

Image.displayName = "CSS(Image)";

