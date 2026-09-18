export interface LinkBoardEndpoint {
  id: string;
  label: string;
  disabled?: boolean;
}

export interface LinkBoardConnection {
  source: string;
  target: string;
}
