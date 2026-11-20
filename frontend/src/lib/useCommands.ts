import { useEffect, useState } from "react";
import { api, type AgentCommand } from "./api";

/** The agent's commands; read again each time its run ends, as every run reports its list afresh. */
export function useCommands(agentId: string, running: boolean): AgentCommand[] {
  const [list, setList] = useState<AgentCommand[]>([]);
  useEffect(() => {
    let stale = false;
    api.commands(agentId).then((l) => !stale && setList(l)).catch(() => {});
    return () => { stale = true; };
  }, [agentId, running]);
  return list;
}
