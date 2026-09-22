import { requestImage } from "../commands";
import { useEffect, useState } from "react";
export function RequestImage({ url }: { url: string }): JSX.Element {
  const [image, setImage] = useState("");
  const [error, setError] = useState("");
  useEffect(() => {
    let cancelled = false;
    requestImage(url)
      .then((value) => {
        if (!cancelled) setImage(value);
      })
      .catch((reason: unknown) => {
        if (!cancelled) setError(String(reason));
      });
    return () => {
      cancelled = true;
    };
  }, [url]);
  return image ? (
    <img src={image} className="requestImage" alt="Request attachment" />
  ) : (
    <p role="status">{error || "Loading image…"}</p>
  );
}
