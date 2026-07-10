import { createServer, type Server } from "node:http";

export const MOCK_CAS_PORT = 14900;

export function startMockCas(port: number = MOCK_CAS_PORT): Server {
  return createServer((req, res) => {
    const url = new URL(req.url!, `http://localhost:${port}`);

    if (url.pathname === "/cas/serviceValidate") {
      const ticket = url.searchParams.get("ticket") ?? "";

      if (ticket === "fail") {
        res.writeHead(200, { "content-type": "text/xml" });
        res.end(
          `<cas:serviceResponse xmlns:cas="http://www.yale.edu/tp/cas">` +
            `<cas:authenticationFailure code="INVALID_TICKET">` +
            `mock: ticket invalid` +
            `</cas:authenticationFailure>` +
            `</cas:serviceResponse>`,
        );
        return;
      }

      const user = ticket.replace(/^ticket-/, "") || "cas-user";
      res.writeHead(200, { "content-type": "text/xml" });
      res.end(
        `<cas:serviceResponse xmlns:cas="http://www.yale.edu/tp/cas">` +
          `<cas:authenticationSuccess>` +
          `<cas:user>${user}</cas:user>` +
          `</cas:authenticationSuccess>` +
          `</cas:serviceResponse>`,
      );
      return;
    }

    res.writeHead(404);
    res.end();
  }).listen(port);
}
