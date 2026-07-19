import React, { createContext, useContext, useEffect, useMemo, type PropsWithChildren, type ReactElement } from "react";
import { AnnotationContext, type AnnotationKey, type AnnotationPrimitive } from "./types.js";
import type { ChillBrowser } from "./runtime.js";

const ClientContext = createContext<ChillBrowser | undefined>(undefined);
const AnnotationReactContext = createContext(AnnotationContext.empty());
export function ChillProvider({ client, children }: PropsWithChildren<{ client: ChillBrowser }>): ReactElement { return React.createElement(ClientContext.Provider, { value: client }, children); }
export function ChillAnnotation<T extends AnnotationPrimitive>({ annotation, value, children }: PropsWithChildren<{ annotation: AnnotationKey<T>; value: T }>): ReactElement { const parent = useContext(AnnotationReactContext); const context = useMemo(() => parent.with(annotation, value), [parent, annotation, value]); return React.createElement(AnnotationReactContext.Provider, { value: context }, children); }
export function ChillPage({ name, children }: PropsWithChildren<{ name: string }>): ReactElement { const client = useChill(); useEffect(() => { client.startPage(name); }, [client, name]); return React.createElement(React.Fragment, undefined, children); }
export function useChill(): ChillBrowser { const client = useContext(ClientContext); if (!client) throw new Error("useChill must be used within ChillProvider"); return client; }
export function useChillAction(name: string): () => void { const client = useChill(); const context = useContext(AnnotationReactContext); return () => { client.setContext(context); client.action(name); }; }
