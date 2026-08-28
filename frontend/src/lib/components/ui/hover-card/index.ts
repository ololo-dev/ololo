import { LinkPreview as HoverCardPrimitive } from "bits-ui";
import Content from "./hover-card-content.svelte";

// shadcn-svelte hover-card, built on bits-ui's LinkPreview primitive (the
// hover-card of the 0.21 line).
const Root = HoverCardPrimitive.Root;
const Trigger = HoverCardPrimitive.Trigger;

export {
  Root,
  Trigger,
  Content,
  //
  Root as HoverCard,
  Trigger as HoverCardTrigger,
  Content as HoverCardContent,
};
