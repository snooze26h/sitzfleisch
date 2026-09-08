import { createBlocker } from "./worker.js";

const blocker = createBlocker({ chromeApi: chrome });
void blocker.start();
