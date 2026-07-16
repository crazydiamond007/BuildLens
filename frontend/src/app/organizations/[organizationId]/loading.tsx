export default function Loading() {
  return <div className="loadingPage"><div className="loadingLine wide" /><div className="loadingLine medium" /><div className="loadingGrid">{Array.from({ length: 4 }, (_, index) => <div className="loadingCard" key={index} />)}</div><div className="loadingTable" /></div>;
}
