declare namespace JSX {
    interface Element {}
    interface IntrinsicElements {
        div: { className?: string; id?: string; hidden?: boolean };
        span: { title?: string };
    }
}
const a = <div className="x" ></div>;
const b = <span></span>;
const c = <di
