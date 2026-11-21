/* The vdom seam — React + htm arrive as UMD globals from the CDN;
 * every module imports the tagged-template factory from here so no
 * file spells the bind twice. */
export const html = htm.bind(React.createElement);
export const { useState, useEffect } = React;
