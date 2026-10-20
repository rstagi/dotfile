// snapcn UI Core · snapcn — https://snapcn.dev/docs/components
import { createContext, useContext } from "react";

const ItemHostContext = createContext(null);

export const ItemHostProvider = ItemHostContext.Provider;

export function Item(
  {
    index,
    primary = true,
    children
  }
) {
  const host = useContext(ItemHostContext);
  return host && primary ? host(index, children) : children;
}
