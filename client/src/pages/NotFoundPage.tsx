import { Link } from "react-router";

export function NotFoundPage() {
  return (
    <div className="page">
      <h2>Not found</h2>
      <p className="muted">There's nothing at this address.</p>
      <Link to="/">Go to your campaigns</Link>
    </div>
  );
}
