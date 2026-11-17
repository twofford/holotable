// The test maps from fixtures/, bundled into the client so they can be opened on
// an iPad without copying files to it. `?url` makes Vite copy each file into the
// build and give us its URL instead of its contents.

import smallUrl from "../../../fixtures/small-bodega.uvtt?url";
import typicalUrl from "../../../fixtures/typical-safehouse.uvtt?url";
import largeUrl from "../../../fixtures/large-arcology.uvtt?url";

export interface SampleMap {
  readonly label: string;
  readonly name: string;
  readonly url: string;
}

export const SAMPLE_MAPS: readonly SampleMap[] = [
  { label: "Small", name: "small-bodega.uvtt", url: smallUrl },
  { label: "Typical", name: "typical-safehouse.uvtt", url: typicalUrl },
  { label: "Large", name: "large-arcology.uvtt", url: largeUrl },
];
