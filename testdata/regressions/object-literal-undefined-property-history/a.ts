declare const condition: boolean;

const earlier = condition ? { hoveredNode: 1 } : {};
void earlier;

const sockets = condition
  ? {
      selectedNode: { type: 0 },
      selectionRayOrigin: { type: 0 },
      selectionPoint: { type: 0 },
      controllerIndex: { type: 0 },
      event: { type: 0 },
    }
  : { hoveredNode: { type: 0 } };

const target: { [id: string]: { type: number } } = sockets;
void target;
