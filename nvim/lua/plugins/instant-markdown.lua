return {
  "instant-markdown/instant-markdown.nvim",
  dependencies = { "nvim-lua/plenary.nvim" },
  ft = "markdown",
  opts = {
    mathjax = true,
    mermaid = true,
  },
  config = function(_, opts)
    require("instant-markdown").setup(opts)

    -- The plugin only refreshes on insert-mode events, so opening a
    -- second markdown buffer leaves the browser stuck on the first one.
    -- Push the current buffer to the daemon on buffer enter / save.
    vim.api.nvim_create_autocmd({ "BufEnter", "BufWritePost" }, {
      group = vim.api.nvim_create_augroup("InstantMarkdownSwitch", { clear = true }),
      pattern = { "*.md", "*.markdown" },
      callback = function(args)
        local body = table.concat(vim.api.nvim_buf_get_lines(args.buf, 0, -1, false), "\n")
        require("plenary.curl").put({
          url = "http://localhost:8090",
          raw_body = body,
          headers = { ["Content-Type"] = "text/plain" },
          insecure = true,
          callback = function() end,
        })
      end,
    })
  end,
}
